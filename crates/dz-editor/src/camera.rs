//! Cameras and the projection helpers shared by rendering and picking.
use eframe::egui::{pos2, Pos2, Rect};

pub type V3 = [f32; 3];

#[inline]
pub fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
#[inline]
pub fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
#[inline]
pub fn mul(a: V3, k: f32) -> V3 {
    [a[0] * k, a[1] * k, a[2] * k]
}
#[inline]
pub fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
#[inline]
pub fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
#[inline]
pub fn norm(a: V3) -> V3 {
    mul(a, 1.0 / dot(a, a).sqrt().max(1e-9))
}

/// Free 3D camera. Yaw is clockwise from north, pitch up from level, both in radians.
#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub pos: V3,
    pub yaw: f32,
    pub pitch: f32,
    pub fov: f32,
    pub speed: f32,
}

impl Camera {
    pub fn forward(&self) -> V3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        [sy * cp, sp, cy * cp]
    }
    pub fn right(&self) -> V3 {
        let (sy, cy) = self.yaw.sin_cos();
        [cy, 0.0, -sy]
    }
    pub fn look_at(&mut self, target: V3) {
        let f = norm(sub(target, self.pos));
        self.pitch = f[1].clamp(-1.0, 1.0).asin();
        self.yaw = f[0].atan2(f[2]);
    }
}

/// One rendered view: matrices plus the screen rectangle.
#[derive(Clone, Copy, Debug)]
pub struct View {
    pub rect: Rect,
    pub vp: [f32; 16],
    pub eye: V3,
    pub forward: V3,
    pub right: V3,
    pub up: V3,
    pub ortho: bool,
    /// Metres per point of an orthographic view.
    pub mpp: f32,
    pub tan_half_fov: f32,
    pub aspect: f32,
}

impl View {
    pub fn perspective(cam: &Camera, rect: Rect, ground_clearance: f32) -> Self {
        let aspect = (rect.width() / rect.height().max(1.0)).max(0.01);
        let near = (ground_clearance * 0.03).clamp(0.25, 40.0);
        let far = 80_000.0f32;
        let f = cam.forward();
        let r = cam.right();
        let u = cross(f, r);
        let t = (cam.fov * 0.5).tan();
        let (x, y) = (1.0 / (t * aspect), 1.0 / t);
        let (a, b) = (-(far + near) / (far - near), -2.0 * far * near / (far - near));
        let e = cam.pos;
        // Columns of projection * view.
        let vp = [
            x * r[0], y * u[0], a * -f[0], f[0],
            x * r[1], y * u[1], a * -f[1], f[1],
            x * r[2], y * u[2], a * -f[2], f[2],
            -x * dot(r, e), -y * dot(u, e), a * dot(f, e) + b, -dot(f, e),
        ];
        Self { rect, vp, eye: e, forward: f, right: r, up: u, ortho: false, mpp: 0.0, tan_half_fov: t, aspect }
    }

    /// Looks straight down; `center` is the map position in the middle of the rectangle.
    pub fn top(center: [f32; 2], mpp: f32, rect: Rect) -> Self {
        let (w, h) = (rect.width() * mpp, rect.height() * mpp);
        let (sx, sz) = (2.0 / w.max(1.0), 2.0 / h.max(1.0));
        let dz = -1.0 / 20000.0;
        let vp = [
            sx, 0.0, 0.0, 0.0,
            0.0, 0.0, dz, 0.0,
            0.0, sz, 0.0, 0.0,
            -center[0] * sx, -center[1] * sz, 0.0, 1.0,
        ];
        let eye = [center[0], (rect.height() * mpp).max(50.0) * 0.9, center[1]];
        Self { rect, vp, eye, forward: [0.0, -1.0, 0.0], right: [1.0, 0.0, 0.0], up: [0.0, 0.0, 1.0], ortho: true, mpp, tan_half_fov: 1.0, aspect: rect.width() / rect.height().max(1.0) }
    }

    /// Screen position of a world point; `None` when it is behind the camera.
    pub fn project(&self, p: V3) -> Option<Pos2> {
        let m = &self.vp;
        let cx = m[0] * p[0] + m[4] * p[1] + m[8] * p[2] + m[12];
        let cy = m[1] * p[0] + m[5] * p[1] + m[9] * p[2] + m[13];
        let w = m[3] * p[0] + m[7] * p[1] + m[11] * p[2] + m[15];
        if w <= 1e-4 {
            return None;
        }
        Some(pos2(
            self.rect.left() + (cx / w * 0.5 + 0.5) * self.rect.width(),
            self.rect.top() + (0.5 - cy / w * 0.5) * self.rect.height(),
        ))
    }

    /// World ray through a screen position.
    pub fn ray(&self, s: Pos2) -> (V3, V3) {
        let nx = (s.x - self.rect.left()) / self.rect.width() * 2.0 - 1.0;
        let ny = 1.0 - (s.y - self.rect.top()) / self.rect.height() * 2.0;
        if self.ortho {
            let o = add(add(self.eye, mul(self.right, nx * self.rect.width() * 0.5 * self.mpp)), mul(self.up, ny * self.rect.height() * 0.5 * self.mpp));
            ([o[0], 9000.0, o[2]], [0.0, -1.0, 0.0])
        } else {
            let d = add(add(self.forward, mul(self.right, nx * self.tan_half_fov * self.aspect)), mul(self.up, ny * self.tan_half_fov));
            (self.eye, norm(d))
        }
    }

    /// Frustum planes (a, b, c, d) with the inside positive.
    pub fn planes(&self) -> [[f32; 4]; 6] {
        let m = &self.vp;
        let row = |i: usize| [m[i], m[4 + i], m[8 + i], m[12 + i]];
        let (r0, r1, r2, r3) = (row(0), row(1), row(2), row(3));
        let c = |a: [f32; 4], s: f32| [r3[0] + a[0] * s, r3[1] + a[1] * s, r3[2] + a[2] * s, r3[3] + a[3] * s];
        [c(r0, 1.0), c(r0, -1.0), c(r1, 1.0), c(r1, -1.0), c(r2, 1.0), c(r2, -1.0)]
    }
}

#[inline]
pub fn box_outside(planes: &[[f32; 4]; 6], mn: V3, mx: V3) -> bool {
    planes.iter().any(|p| {
        let x = if p[0] > 0.0 { mx[0] } else { mn[0] };
        let y = if p[1] > 0.0 { mx[1] } else { mn[1] };
        let z = if p[2] > 0.0 { mx[2] } else { mn[2] };
        p[0] * x + p[1] * y + p[2] * z + p[3] < 0.0
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Rect, Vec2};

    fn rect() -> Rect {
        Rect::from_min_size(pos2(10.0, 20.0), Vec2::new(800.0, 600.0))
    }

    #[test]
    fn perspective_projects_ahead_to_center_and_east_to_the_right() {
        let cam = Camera { pos: [0.0, 10.0, 0.0], yaw: 0.0, pitch: 0.0, fov: 1.2, speed: 1.0 };
        let v = View::perspective(&cam, rect(), 10.0);
        let c = v.project([0.0, 10.0, 100.0]).unwrap();
        assert!((c.x - 410.0).abs() < 0.01 && (c.y - 320.0).abs() < 0.01);
        assert!(v.project([50.0, 10.0, 100.0]).unwrap().x > c.x);
        assert!(v.project([0.0, 60.0, 100.0]).unwrap().y < c.y);
        assert!(v.project([0.0, 10.0, -100.0]).is_none());
    }

    #[test]
    fn rays_invert_projection() {
        let mut cam = Camera { pos: [5.0, 40.0, -3.0], yaw: 0.7, pitch: -0.3, fov: 1.1, speed: 1.0 };
        cam.look_at([100.0, 0.0, 200.0]);
        let v = View::perspective(&cam, rect(), 10.0);
        let p = [120.0, 3.0, 180.0];
        let s = v.project(p).unwrap();
        let (o, d) = v.ray(s);
        let t = dot(sub(p, o), d);
        let hit = add(o, mul(d, t));
        assert!(dot(sub(hit, p), sub(hit, p)).sqrt() < 0.05);
    }

    #[test]
    fn top_view_maps_north_up() {
        let v = View::top([1000.0, 1000.0], 2.0, rect());
        let c = v.project([1000.0, 5.0, 1000.0]).unwrap();
        assert!((c.x - 410.0).abs() < 0.01 && (c.y - 320.0).abs() < 0.01);
        assert!(v.project([1000.0, 5.0, 1100.0]).unwrap().y < c.y);
        assert!(v.project([1100.0, 5.0, 1000.0]).unwrap().x > c.x);
        let (o, _) = v.ray(pos2(c.x + 100.0, c.y));
        assert!((o[0] - 1200.0).abs() < 0.01);
    }

    #[test]
    fn frustum_keeps_visible_boxes() {
        let cam = Camera { pos: [0.0, 10.0, 0.0], yaw: 0.0, pitch: 0.0, fov: 1.2, speed: 1.0 };
        let v = View::perspective(&cam, rect(), 10.0);
        let p = v.planes();
        assert!(!box_outside(&p, [-5.0, 0.0, 90.0], [5.0, 20.0, 110.0]));
        assert!(box_outside(&p, [-5.0, 0.0, -110.0], [5.0, 20.0, -90.0]));
    }
}
