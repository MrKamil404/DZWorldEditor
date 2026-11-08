//! OpenGL renderer: CDLOD terrain sampled from a height texture, instanced objects.
//!
//! The UI thread prepares plain data (`Frame`, queued uploads) and this module only touches
//! GL inside egui paint callbacks. Sculpting therefore only re-uploads the changed rectangle
//! and never rebuilds meshes.
use crate::camera::{box_outside, View};
use dz_core::terrain::{Heightmap, Rect};
use eframe::egui_glow;
use eframe::glow::{self, HasContext};
use std::collections::HashMap;
use std::sync::Arc;

pub const PATCH: usize = 32;

const TERRAIN_VS: &str = r#"#version 330 core
layout(location=0) in vec2 a_grid;
layout(location=1) in vec4 i_patch;
uniform mat4 u_vp;
uniform sampler2D u_height;
uniform vec2 u_size;
uniform float u_cell;
uniform vec3 u_eye;
uniform float u_range[20];
out vec3 v_world;
float H(vec2 t){ return texture(u_height, (t + 0.5) / u_size).r; }
void main(){
  vec2 t = i_patch.xy + a_grid * i_patch.z;
  float y0 = H(min(t, u_size - 1.0));
  float d = distance(vec3(t.x * u_cell, y0, t.y * u_cell), u_eye);
  float r = u_range[int(i_patch.w)];
  float morph = clamp((d - r * 0.72) / (r * 0.28), 0.0, 1.0);
  vec2 g = a_grid - fract(a_grid * 0.5) * 2.0 * morph;
  vec2 m = i_patch.xy + g * i_patch.z;
  float y = H(min(m, u_size - 1.0));
  v_world = vec3(m.x * u_cell, y, m.y * u_cell);
  gl_Position = u_vp * vec4(v_world, 1.0);
}"#;

const TERRAIN_FS: &str = r#"#version 330 core
in vec3 v_world;
out vec4 o;
uniform sampler2D u_height;
uniform vec2 u_size;
uniform float u_cell;
uniform float u_world;
uniform int u_mode;
uniform vec3 u_eye;
uniform sampler2D u_sat, u_detail, u_palette;
uniform int u_has_sat;
uniform vec4 u_detail_rect;
uniform usampler2D u_mask;
uniform vec2 u_mask_size;
uniform sampler2DArray u_layers;
uniform float u_tile;
uniform vec4 u_brush;
uniform vec4 u_brush_color;
uniform vec4 u_grid;
uniform float u_sea;
uniform vec2 u_height_range;
uniform int u_ortho;
const vec3 SUN = normalize(vec3(-0.45, 0.8, 0.4));
const vec3 FOG = vec3(0.66, 0.74, 0.84);

float H(vec2 p){ return texture(u_height, (p / u_cell + 0.5) / u_size).r; }
vec3 normalAt(vec2 p){
  float d = u_cell;
  float x = H(p + vec2(d,0)) - H(p - vec2(d,0));
  float z = H(p + vec2(0,d)) - H(p - vec2(0,d));
  return normalize(vec3(-x, 2.0 * d, -z));
}
uint maskAt(ivec2 p){ return texelFetch(u_mask, clamp(p, ivec2(0), ivec2(u_mask_size) - 1), 0).r; }
vec3 pal(uint i){ return texelFetch(u_palette, ivec2(int(i), 0), 0).rgb; }
vec3 avg(uint i){ return texelFetch(u_palette, ivec2(int(i), 1), 0).rgb; }

vec3 ramp(float t){
  t = clamp(t, 0.0, 1.0);
  vec3 a = vec3(0.13,0.34,0.20), b = vec3(0.55,0.62,0.30), c = vec3(0.56,0.42,0.28), d = vec3(0.95,0.95,0.97);
  if (t < 0.33) return mix(a,b,t/0.33);
  if (t < 0.66) return mix(b,c,(t-0.33)/0.33);
  return mix(c,d,(t-0.66)/0.34);
}

void main(){
  vec2 uv = vec2(v_world.x, u_world - v_world.z) / u_world;
  vec3 n = normalAt(v_world.xz);
  float dist = distance(v_world, u_eye);
  vec3 col;
  // Surface blending: four nearest mask pixels.
  vec2 mp = uv * u_mask_size - 0.5;
  ivec2 ip = ivec2(floor(mp));
  vec2 f = fract(mp);
  uint a = maskAt(ip), b = maskAt(ip + ivec2(1,0)), c = maskAt(ip + ivec2(0,1)), d = maskAt(ip + ivec2(1,1));
  float wa = (1.0-f.x)*(1.0-f.y), wb = f.x*(1.0-f.y), wc = (1.0-f.x)*f.y, wd = f.x*f.y;
  vec3 palette = pal(a)*wa + pal(b)*wb + pal(c)*wc + pal(d)*wd;

  vec3 sat = palette;
  if (u_has_sat != 0) {
    sat = texture(u_sat, uv).rgb;
    vec2 q = (uv - u_detail_rect.xy) / u_detail_rect.zw;
    vec2 e = smoothstep(vec2(0.0), vec2(0.04), q) * smoothstep(vec2(0.0), vec2(0.04), 1.0 - q);
    float k = e.x * e.y;
    if (k > 0.0) sat = mix(sat, texture(u_detail, q).rgb, k);
  }
  vec3 light = vec3(0.56,0.60,0.68) * (0.6 + 0.4*n.y) + vec3(1.0,0.96,0.88) * max(dot(n, SUN), 0.0) * 0.82;

  if (u_mode == 0) { // lit, like in the game
    vec3 alb = sat;
    float near = 1.0 - smoothstep(u_ortho != 0 ? 0.0 : 90.0, u_ortho != 0 ? 700.0 : 520.0, dist);
    if (near > 0.002) {
      vec2 t = v_world.xz * u_tile;
      vec2 dx = dFdx(t), dy = dFdy(t);
      vec3 s = textureGrad(u_layers, vec3(t, float(a)), dx, dy).rgb / max(avg(a), vec3(0.05)) * wa
             + textureGrad(u_layers, vec3(t, float(b)), dx, dy).rgb / max(avg(b), vec3(0.05)) * wb
             + textureGrad(u_layers, vec3(t, float(c)), dx, dy).rgb / max(avg(c), vec3(0.05)) * wc
             + textureGrad(u_layers, vec3(t, float(d)), dx, dy).rgb / max(avg(d), vec3(0.05)) * wd;
      alb = mix(sat, sat * s, near);
    }
    col = alb * light;
  } else if (u_mode == 1) { // satellite
    col = sat;
  } else if (u_mode == 2) { // mask, hard edges
    col = pal(maskAt(ivec2(floor(uv * u_mask_size))));
  } else if (u_mode == 3) { // hillshade
    col = vec3(0.62) * light;
  } else if (u_mode == 4) { // height
    float t = (v_world.y - u_height_range.x) / max(u_height_range.y - u_height_range.x, 1.0);
    col = ramp(t) * (0.45 + 0.55 * max(dot(n, SUN), 0.0) / 0.8);
  } else { // slope
    float s = degrees(acos(clamp(n.y, -1.0, 1.0)));
    col = mix(vec3(0.15,0.5,0.25), vec3(0.9,0.75,0.2), smoothstep(5.0,20.0,s));
    col = mix(col, vec3(0.85,0.15,0.1), smoothstep(22.0,40.0,s));
  }
  // Contours, land grid and tiles.
  if (u_grid.x > 0.0) {
    vec2 g = abs(fract(v_world.xz / u_grid.x - 0.5) - 0.5) / fwidth(v_world.xz / u_grid.x);
    col = mix(col, vec3(0.1,0.15,0.2), (1.0 - clamp(min(g.x,g.y), 0.0, 1.0)) * 0.35);
  }
  if (u_grid.y > 0.0) {
    vec2 g = abs(fract(v_world.xz / u_grid.y - 0.5) - 0.5) / fwidth(v_world.xz / u_grid.y);
    col = mix(col, vec3(1.0,0.7,0.2), (1.0 - clamp(min(g.x,g.y), 0.0, 1.0)) * 0.5);
  }
  if (u_grid.z > 0.0) {
    float l = v_world.y / u_grid.z;
    float g = abs(fract(l - 0.5) - 0.5) / fwidth(l);
    col = mix(col, vec3(0.2,0.1,0.05), (1.0 - clamp(g, 0.0, 1.0)) * 0.45);
  }
  if (u_brush.z > 0.0) {
    float bd = distance(v_world.xz, u_brush.xy);
    float px = fwidth(bd) * 1.5;
    float ring = 1.0 - clamp(abs(bd - u_brush.z) / px, 0.0, 1.0);
    float inner = 1.0 - clamp(abs(bd - u_brush.z * (1.0 - u_brush.w)) / px, 0.0, 1.0);
    float fill = bd < u_brush.z ? 0.10 : 0.0;
    col = mix(col, u_brush_color.rgb, max(max(ring, inner * 0.5), fill));
  }
  if (u_ortho == 0 && u_mode <= 1) col = mix(col, FOG, 1.0 - exp(-dist * 0.00007));
  o = vec4(col, 1.0);
}"#;

const WATER_VS: &str = r#"#version 330 core
layout(location=0) in vec2 a_pos;
uniform mat4 u_vp; uniform vec4 u_area; uniform float u_sea;
out vec3 v_world;
void main(){ v_world = vec3(u_area.x + a_pos.x * u_area.z, u_sea, u_area.y + a_pos.y * u_area.w); gl_Position = u_vp * vec4(v_world, 1.0); }"#;

const WATER_FS: &str = r#"#version 330 core
in vec3 v_world; out vec4 o; uniform vec3 u_eye;
void main(){
  vec3 v = normalize(u_eye - v_world);
  float fres = pow(1.0 - max(v.y, 0.0), 3.0);
  vec3 col = mix(vec3(0.05,0.2,0.3), vec3(0.55,0.68,0.8), fres);
  float dist = distance(v_world, u_eye);
  col = mix(col, vec3(0.66,0.74,0.84), 1.0 - exp(-dist * 0.00007));
  o = vec4(col, 0.82);
}"#;

const MESH_VS: &str = r#"#version 330 core
layout(location=0) in vec3 a_pos;
layout(location=1) in vec3 a_normal;
layout(location=2) in vec2 a_uv;
layout(location=3) in vec4 i_r0;
layout(location=4) in vec4 i_r1;
layout(location=5) in vec4 i_r2;
layout(location=6) in vec4 i_tint;
uniform mat4 u_vp;
out vec3 v_n; out vec2 v_uv; out vec3 v_world; flat out vec4 v_tint;
void main(){
  vec4 p = vec4(a_pos, 1.0);
  v_world = vec3(dot(i_r0, p), dot(i_r1, p), dot(i_r2, p));
  vec4 n = vec4(a_normal, 0.0);
  v_n = normalize(vec3(dot(i_r0, n), dot(i_r1, n), dot(i_r2, n)));
  v_uv = a_uv; v_tint = i_tint;
  gl_Position = u_vp * vec4(v_world, 1.0);
}"#;

const MESH_FS: &str = r#"#version 330 core
in vec3 v_n; in vec2 v_uv; in vec3 v_world; flat in vec4 v_tint;
out vec4 o;
uniform sampler2D u_tex; uniform int u_textured; uniform vec3 u_eye; uniform float u_alpha;
const vec3 SUN = normalize(vec3(-0.45, 0.8, 0.4));
void main(){
  vec4 c = vec4(0.62,0.62,0.6,1.0);
  if (u_textured != 0) { c = texture(u_tex, v_uv); if (c.a < mix(0.42, 0.1, clamp(distance(v_world, u_eye) / 350.0, 0.0, 1.0))) discard; }
  else if (v_tint.a > 0.0 && v_tint.a < 0.99) { c.rgb = v_tint.rgb; }
  vec3 n = normalize(v_n);
  float l = abs(dot(n, SUN));
  vec3 lit = c.rgb * (vec3(0.42,0.46,0.52) + vec3(1.0,0.95,0.86) * l * 0.75);
  if (v_tint.a >= 0.99) lit = mix(lit, v_tint.rgb, 0.55);
  float dist = distance(v_world, u_eye);
  lit = mix(lit, vec3(0.66,0.74,0.84), 1.0 - exp(-dist * 0.00007));
  o = vec4(lit, v_tint.a >= 0.99 ? 1.0 : u_alpha);
}"#;

const SKY_VS: &str = r#"#version 330 core
layout(location=0) in vec2 a_pos;
out vec2 v;
void main(){ v = a_pos * 2.0 - 1.0; gl_Position = vec4(v, 1.0, 1.0); }"#;

const SKY_FS: &str = r#"#version 330 core
in vec2 v; out vec4 o;
uniform vec3 u_f, u_r, u_u; uniform vec2 u_t;
void main(){
  vec3 d = normalize(u_f + u_r * v.x * u_t.x + u_u * v.y * u_t.y);
  float h = clamp(d.y, -0.05, 1.0);
  vec3 c = mix(vec3(0.66,0.74,0.84), vec3(0.24,0.44,0.72), pow(max(h, 0.0), 0.55));
  float s = max(dot(d, normalize(vec3(-0.45, 0.8, 0.4))), 0.0);
  c += vec3(1.0,0.9,0.75) * (pow(s, 400.0) * 1.2 + pow(s, 12.0) * 0.08);
  o = vec4(c, 1.0);
}"#;

const IMPOSTOR_VS: &str = r#"#version 330 core
layout(location=0) in vec2 a_pos;
layout(location=1) in vec4 i_pos;
layout(location=2) in vec4 i_fit;
uniform mat4 u_vp; uniform vec3 u_right;
out vec3 v_uv; out vec3 v_world;
void main(){
  float s = i_pos.w;
  vec2 a = a_pos - 0.5;
  vec3 r = normalize(vec3(u_right.x, 0.0, u_right.z));
  v_world = i_pos.xyz + r * (a.x * s + i_fit.x) + vec3(0.0, 1.0, 0.0) * (a.y * s + i_fit.y);
  v_uv = vec3(a.x + 0.5, 0.5 - a.y, i_fit.z);
  gl_Position = u_vp * vec4(v_world, 1.0);
}"#;

const IMPOSTOR_FS: &str = r#"#version 330 core
in vec3 v_uv; in vec3 v_world; out vec4 o;
uniform sampler2DArray u_tex; uniform vec3 u_eye;
void main(){
  vec4 c = texture(u_tex, v_uv);
  if (c.a < 0.5) discard;
  vec3 lit = c.rgb * vec3(0.95,0.95,0.92);
  float dist = distance(v_world, u_eye);
  o = vec4(mix(lit, vec3(0.66,0.74,0.84), 1.0 - exp(-dist * 0.00007)), 1.0);
}"#;

#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub enum Shading {
    Lit,
    Satellite,
    Mask,
    Hillshade,
    Height,
    Slope,
}
impl Shading {
    pub const ALL: [Shading; 6] = [Shading::Lit, Shading::Satellite, Shading::Mask, Shading::Hillshade, Shading::Height, Shading::Slope];
    pub fn name(self) -> &'static str {
        match self {
            Shading::Lit => "Lit",
            Shading::Satellite => "Satellite",
            Shading::Mask => "Mask",
            Shading::Hillshade => "Hillshade",
            Shading::Height => "Height",
            Shading::Slope => "Slope",
        }
    }
}

#[derive(Clone, Copy)]
pub struct BrushView {
    pub x: f32,
    pub z: f32,
    pub radius: f32,
    pub falloff: f32,
    pub color: [f32; 3],
}

/// Height range and pyramid used to choose terrain patches.
pub struct TerrainLod {
    levels: Vec<Vec<[f32; 2]>>,
    dims: Vec<(usize, usize)>,
    pub cell: f32,
}

impl TerrainLod {
    pub fn build(h: &Heightmap) -> Self {
        let (w0, h0) = (h.cols.div_ceil(PATCH), h.rows.div_ceil(PATCH));
        let mut level0 = vec![[f32::MAX, f32::MIN]; w0 * h0];
        for j in 0..h.rows {
            for i in 0..h.cols {
                let v = h.at(i, j);
                let e = &mut level0[(j / PATCH) * w0 + i / PATCH];
                e[0] = e[0].min(v);
                e[1] = e[1].max(v);
            }
        }
        let mut levels = vec![level0];
        let mut dims = vec![(w0, h0)];
        while dims.last().unwrap().0 > 1 || dims.last().unwrap().1 > 1 {
            let (pw, ph) = *dims.last().unwrap();
            let (w, h2) = (pw.div_ceil(2), ph.div_ceil(2));
            let prev = levels.last().unwrap();
            let mut next = vec![[f32::MAX, f32::MIN]; w * h2];
            for j in 0..ph {
                for i in 0..pw {
                    let e = &mut next[(j / 2) * w + i / 2];
                    e[0] = e[0].min(prev[j * pw + i][0]);
                    e[1] = e[1].max(prev[j * pw + i][1]);
                }
            }
            levels.push(next);
            dims.push((w, h2));
        }
        Self { levels, dims, cell: h.cell }
    }

    /// Refreshes the pyramid over a changed rectangle.
    pub fn update(&mut self, h: &Heightmap, r: Rect) {
        let (w0, _) = self.dims[0];
        let (gx0, gy0, gx1, gy1) = (r.x0 / PATCH, r.y0 / PATCH, r.x1 / PATCH, r.y1 / PATCH);
        for gy in gy0..=gy1 {
            for gx in gx0..=gx1 {
                let (mut lo, mut hi) = (f32::MAX, f32::MIN);
                for j in gy * PATCH..((gy + 1) * PATCH).min(h.rows) {
                    for i in gx * PATCH..((gx + 1) * PATCH).min(h.cols) {
                        let v = h.at(i, j);
                        lo = lo.min(v);
                        hi = hi.max(v);
                    }
                }
                self.levels[0][gy * w0 + gx] = [lo, hi];
            }
        }
        let (mut ax, mut ay, mut bx, mut by) = (gx0, gy0, gx1, gy1);
        for l in 1..self.levels.len() {
            let (pw, ph) = self.dims[l - 1];
            let (w, _) = self.dims[l];
            ax /= 2;
            ay /= 2;
            bx /= 2;
            by /= 2;
            for y in ay..=by {
                for x in ax..=bx {
                    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
                    for cy in y * 2..(y * 2 + 2).min(ph) {
                        for cx in x * 2..(x * 2 + 2).min(pw) {
                            let e = self.levels[l - 1][cy * pw + cx];
                            lo = lo.min(e[0]);
                            hi = hi.max(e[1]);
                        }
                    }
                    self.levels[l][y * w + x] = [lo, hi];
                }
            }
        }
    }

    pub fn ranges(&self, quality: f32) -> Vec<f32> {
        let base = PATCH as f32 * self.cell * 3.0 * quality;
        (0..20).map(|l| base * (1u32 << l.min(18)) as f32).collect()
    }

    /// Patches to draw: (texel x, texel z, step, level).
    pub fn select(&self, view: &View, quality: f32) -> Vec<[f32; 4]> {
        let planes = view.planes();
        let ranges = self.ranges(quality);
        let mut out = Vec::with_capacity(1024);
        let top = self.levels.len() - 1;
        let (w, h) = self.dims[top];
        for z in 0..h {
            for x in 0..w {
                self.visit(top, x, z, view, &planes, &ranges, &mut out);
            }
        }
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn visit(&self, level: usize, x: usize, z: usize, view: &View, planes: &[[f32; 4]; 6], ranges: &[f32], out: &mut Vec<[f32; 4]>) {
        let (w, _) = self.dims[level];
        let e = self.levels[level][z * w + x];
        let texels = (PATCH << level) as f32;
        let size = texels * self.cell;
        let (x0, z0) = (x as f32 * size, z as f32 * size);
        let (mn, mx) = ([x0, e[0], z0], [x0 + size, e[1], z0 + size]);
        if box_outside(planes, mn, mx) {
            return;
        }
        let d = {
            let c = view.eye;
            let dx = (mn[0] - c[0]).max(0.0).max(c[0] - mx[0]);
            let dy = (mn[1] - c[1]).max(0.0).max(c[1] - mx[1]);
            let dz = (mn[2] - c[2]).max(0.0).max(c[2] - mx[2]);
            (dx * dx + dy * dy + dz * dz).sqrt()
        };
        if level == 0 || d > ranges[level - 1] {
            out.push([x as f32 * texels, z as f32 * texels, (1u32 << level) as f32, level as f32]);
            return;
        }
        let (cw, ch) = self.dims[level - 1];
        for (cx, cz) in [(x * 2, z * 2), (x * 2 + 1, z * 2), (x * 2, z * 2 + 1), (x * 2 + 1, z * 2 + 1)] {
            if cx < cw && cz < ch {
                self.visit(level - 1, cx, cz, view, planes, ranges, out);
            }
        }
    }
}

/// A model ready for upload.
pub struct MeshData {
    pub key: (u32, u8),
    /// x, y, z, nx, ny, nz, u, v
    pub vertices: Vec<f32>,
    pub indices: Vec<u32>,
    pub batches: Vec<(u32, u32, Option<usize>)>,
    pub textures: Vec<(u32, u32, Vec<u8>)>,
    pub impostor: Option<Vec<(usize, Vec<u8>)>>,
    /// Side length and image centre of the impostor in model metres.
    pub impostor_fit: [f32; 3],
}

pub const IMPOSTOR_SIZE: i32 = 192;
pub const IMPOSTOR_LAYERS: i32 = 512;

struct Model {
    vao: glow::VertexArray,
    vbo: glow::Buffer,
    ebo: glow::Buffer,
    batches: Vec<(u32, u32, Option<glow::Texture>)>,
    textures: Vec<glow::Texture>,
    triangles: u32,
}

/// Draw list of one view.
#[derive(Default, Clone)]
pub struct ObjectDraw {
    /// 16 floats per instance: three matrix rows, tint.
    pub instances: Arc<Vec<f32>>,
    /// (model key, first instance, count)
    pub groups: Arc<Vec<((u32, u8), u32, u32)>>,
    /// Same layout as instances, drawn with the unit box.
    pub boxes: Arc<Vec<f32>>,
    /// 8 floats per instance: x, y, z, size, fit x, fit y, layer, unused.
    pub impostors: Arc<Vec<f32>>,
}

#[derive(Clone)]
pub struct Frame {
    pub view: View,
    pub viewport_key: u8,
    pub patches: Arc<Vec<[f32; 4]>>,
    pub ranges: Arc<Vec<f32>>,
    pub shading: Shading,
    pub wireframe: bool,
    pub brush: Option<BrushView>,
    pub grid: [f32; 3],
    pub sea: f32,
    pub height_range: [f32; 2],
    pub objects: ObjectDraw,
    pub show_water: bool,
    pub world: f32,
    pub tile: f32,
}

enum Upload {
    Reset { cols: i32, rows: i32, cell: f32, data: Vec<f32> },
    Height { x: i32, y: i32, w: i32, h: i32, data: Vec<f32> },
    Mask { width: i32, height: i32, data: Vec<u8> },
    MaskRect { x: i32, y: i32, w: i32, h: i32, data: Vec<u8> },
    Satellite { width: i32, height: i32, rgb: Vec<u8> },
    NoSatellite,
    Detail { width: i32, height: i32, rgb: Vec<u8>, rect: [f32; 4] },
    Palette { colors: Vec<[u8; 3]>, averages: Vec<[u8; 3]> },
    Layer { index: i32, size: i32, rgba: Vec<u8> },
    Mesh(Box<MeshData>),
    ClearModels,
}

struct Programs {
    terrain: glow::Program,
    water: glow::Program,
    mesh: glow::Program,
    impostor: glow::Program,
    sky: glow::Program,
}

#[derive(Default)]
pub struct Renderer {
    uploads: Vec<Upload>,
    programs: Option<Programs>,
    patch_vao: Option<glow::VertexArray>,
    patch_instances: Option<glow::Buffer>,
    index_count: i32,
    quad_vao: Option<glow::VertexArray>,
    quad_vbo: Option<glow::Buffer>,
    box_vao: Option<glow::VertexArray>,
    box_vbo: Option<glow::Buffer>,
    box_ebo: Option<glow::Buffer>,
    impostor_instances: Option<glow::Buffer>,
    mesh_instances: HashMap<u8, glow::Buffer>,
    box_instances: Option<glow::Buffer>,
    height: Option<glow::Texture>,
    mask: Option<glow::Texture>,
    sat: Option<glow::Texture>,
    detail: Option<glow::Texture>,
    detail_rect: [f32; 4],
    has_sat: bool,
    palette: Option<glow::Texture>,
    layers: Option<glow::Texture>,
    impostors: Option<glow::Texture>,
    size: [f32; 2],
    mask_size: [f32; 2],
    cell: f32,
    models: HashMap<(u32, u8), Model>,
    pub impostor_layers: HashMap<u32, i32>,
    next_impostor: i32,
    pub error: Option<String>,
    pub stats: Stats,
    max_tex: i32,
    instances_uploaded: HashMap<u8, *const Vec<f32>>,
}

// Only used on the GL thread; the pointer is an identity check and never dereferenced.
unsafe impl Send for Renderer {}

#[derive(Default, Clone, Copy, Debug)]
pub struct Stats {
    pub triangles: u64,
    pub draw_calls: u32,
    pub patches: u32,
    pub mesh_instances: u32,
    pub impostors: u32,
    pub models: u32,
}

unsafe fn compile(gl: &glow::Context, vs: &str, fs: &str) -> Result<glow::Program, String> {
    let program = gl.create_program()?;
    let mut shaders = Vec::new();
    for (kind, src) in [(glow::VERTEX_SHADER, vs), (glow::FRAGMENT_SHADER, fs)] {
        let s = gl.create_shader(kind)?;
        gl.shader_source(s, src);
        gl.compile_shader(s);
        if !gl.get_shader_compile_status(s) {
            return Err(gl.get_shader_info_log(s));
        }
        gl.attach_shader(program, s);
        shaders.push(s);
    }
    gl.link_program(program);
    if !gl.get_program_link_status(program) {
        return Err(gl.get_program_info_log(program));
    }
    for s in shaders {
        gl.delete_shader(s);
    }
    Ok(program)
}

fn bytes<T>(v: &[T]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, std::mem::size_of_val(v)) }
}

unsafe fn set_filter(gl: &glow::Context, target: u32, min: u32, mag: u32, wrap: u32) {
    gl.tex_parameter_i32(target, glow::TEXTURE_MIN_FILTER, min as i32);
    gl.tex_parameter_i32(target, glow::TEXTURE_MAG_FILTER, mag as i32);
    gl.tex_parameter_i32(target, glow::TEXTURE_WRAP_S, wrap as i32);
    gl.tex_parameter_i32(target, glow::TEXTURE_WRAP_T, wrap as i32);
}

impl Renderer {
    pub fn reset_terrain(&mut self, h: &Heightmap) {
        self.uploads.retain(|u| matches!(u, Upload::Palette { .. } | Upload::Layer { .. }));
        self.uploads.push(Upload::ClearModels);
        self.uploads.push(Upload::Reset { cols: h.cols as i32, rows: h.rows as i32, cell: h.cell, data: h.data.clone() });
    }
    pub fn update_height(&mut self, h: &Heightmap, r: Rect) {
        let mut data = Vec::with_capacity(r.width() * r.height());
        for j in r.y0..=r.y1 {
            data.extend_from_slice(&h.data[j * h.cols + r.x0..=j * h.cols + r.x1]);
        }
        self.uploads.push(Upload::Height { x: r.x0 as i32, y: r.y0 as i32, w: r.width() as i32, h: r.height() as i32, data });
    }
    pub fn set_mask(&mut self, m: &dz_core::terrain::MaskMap) {
        // Textures above 16k would be refused by most drivers.
        let f = m.width.max(m.height).div_ceil(16384).max(1);
        let (w, h) = (m.width / f, m.height / f);
        let data = if f == 1 {
            m.data.clone()
        } else {
            let mut d = Vec::with_capacity(w * h);
            for y in 0..h {
                for x in 0..w {
                    d.push(m.at(x * f, y * f));
                }
            }
            d
        };
        self.uploads.push(Upload::Mask { width: w as i32, height: h as i32, data });
    }
    pub fn update_mask(&mut self, m: &dz_core::terrain::MaskMap, r: Rect) {
        if m.width.max(m.height) > 16384 {
            self.set_mask(m);
            return;
        }
        let mut data = Vec::with_capacity(r.width() * r.height());
        for y in r.y0..=r.y1 {
            data.extend_from_slice(&m.data[y * m.width + r.x0..=y * m.width + r.x1]);
        }
        self.uploads.push(Upload::MaskRect { x: r.x0 as i32, y: r.y0 as i32, w: r.width() as i32, h: r.height() as i32, data });
    }
    pub fn set_satellite(&mut self, s: Option<&dz_core::terrain::RgbImage>) {
        match s {
            Some(s) => {
                let img = if s.width.max(s.height) > 4096 { s.resized(4096, 4096) } else { s.clone() };
                self.uploads.push(Upload::Satellite { width: img.width as i32, height: img.height as i32, rgb: img.data });
            }
            None => self.uploads.push(Upload::NoSatellite),
        }
    }
    /// Sets the high resolution satellite window around a map position.
    pub fn set_detail(&mut self, s: &dz_core::terrain::RgbImage, cx: f32, cz: f32, world: f32) -> [f32; 4] {
        const SIDE: usize = 2048;
        let side = SIDE.min(s.width).min(s.height);
        let px = (cx / world * s.width as f32) as i64 - side as i64 / 2;
        let py = ((1.0 - cz / world) * s.height as f32) as i64 - side as i64 / 2;
        let x0 = px.clamp(0, (s.width - side) as i64) as usize;
        let y0 = py.clamp(0, (s.height - side) as i64) as usize;
        let mut rgb = Vec::with_capacity(side * side * 3);
        for y in y0..y0 + side {
            rgb.extend_from_slice(&s.data[(y * s.width + x0) * 3..(y * s.width + x0 + side) * 3]);
        }
        let rect = [x0 as f32 / s.width as f32, y0 as f32 / s.height as f32, side as f32 / s.width as f32, side as f32 / s.height as f32];
        self.uploads.push(Upload::Detail { width: side as i32, height: side as i32, rgb, rect });
        rect
    }
    pub fn set_palette(&mut self, colors: Vec<[u8; 3]>, averages: Vec<[u8; 3]>) {
        self.uploads.push(Upload::Palette { colors, averages });
    }
    pub fn set_layer(&mut self, index: usize, size: usize, rgba: Vec<u8>) {
        self.uploads.push(Upload::Layer { index: index as i32, size: size as i32, rgba });
    }
    pub fn add_mesh(&mut self, m: MeshData) {
        self.uploads.push(Upload::Mesh(Box::new(m)));
    }
    /// Triangles of a loaded model.
    pub fn model_triangles(&self, key: (u32, u8)) -> Option<u32> {
        self.models.get(&key).map(|m| m.triangles)
    }

    unsafe fn init(&mut self, gl: &glow::Context) -> Result<(), String> {
        if self.programs.is_some() {
            return Ok(());
        }
        self.max_tex = gl.get_parameter_i32(glow::MAX_TEXTURE_SIZE);
        self.programs = Some(Programs {
            terrain: compile(gl, TERRAIN_VS, TERRAIN_FS)?,
            water: compile(gl, WATER_VS, WATER_FS)?,
            mesh: compile(gl, MESH_VS, MESH_FS)?,
            impostor: compile(gl, IMPOSTOR_VS, IMPOSTOR_FS)?,
            sky: compile(gl, SKY_VS, SKY_FS)?,
        });
        // Patch grid.
        let n = PATCH as u32;
        let mut verts = Vec::with_capacity(((n + 1) * (n + 1) * 2) as usize);
        for z in 0..=n {
            for x in 0..=n {
                verts.push(x as f32);
                verts.push(z as f32);
            }
        }
        let mut idx: Vec<u32> = Vec::with_capacity((n * n * 6) as usize);
        for z in 0..n {
            for x in 0..n {
                let a = z * (n + 1) + x;
                let (b, c, d) = (a + 1, a + n + 1, a + n + 2);
                idx.extend_from_slice(&[a, c, b, b, c, d]);
            }
        }
        self.index_count = idx.len() as i32;
        let vao = gl.create_vertex_array()?;
        gl.bind_vertex_array(Some(vao));
        let vbo = gl.create_buffer()?;
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
        gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes(&verts), glow::STATIC_DRAW);
        gl.enable_vertex_attrib_array(0);
        gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, 8, 0);
        let ebo = gl.create_buffer()?;
        gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(ebo));
        gl.buffer_data_u8_slice(glow::ELEMENT_ARRAY_BUFFER, bytes(&idx), glow::STATIC_DRAW);
        let inst = gl.create_buffer()?;
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(inst));
        gl.enable_vertex_attrib_array(1);
        gl.vertex_attrib_pointer_f32(1, 4, glow::FLOAT, false, 16, 0);
        gl.vertex_attrib_divisor(1, 1);
        self.patch_vao = Some(vao);
        self.patch_instances = Some(inst);

        // Quad for water and impostors.
        let quad: [f32; 8] = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 1.0];
        let qvao = gl.create_vertex_array()?;
        gl.bind_vertex_array(Some(qvao));
        let qvbo = gl.create_buffer()?;
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(qvbo));
        gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes(&quad), glow::STATIC_DRAW);
        gl.enable_vertex_attrib_array(0);
        gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, 8, 0);
        self.quad_vao = Some(qvao);
        self.quad_vbo = Some(qvbo);

        // Unit box, y from 0 to 1: position, normal, uv.
        let mut b: Vec<f32> = Vec::new();
        let mut bi: Vec<u32> = Vec::new();
        let faces: [([f32; 3], [f32; 3], [f32; 3]); 6] = [
            ([0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
            ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
            ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
            ([-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
            ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
            ([0.0, 0.0, -1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        ];
        for (n, u, v) in faces {
            let base = (b.len() / 8) as u32;
            for (s, t) in [(-0.5f32, -0.5f32), (0.5, -0.5), (-0.5, 0.5), (0.5, 0.5)] {
                let p = [n[0] * 0.5 + u[0] * s + v[0] * t, n[1] * 0.5 + u[1] * s + v[1] * t + 0.5, n[2] * 0.5 + u[2] * s + v[2] * t];
                b.extend_from_slice(&[p[0], p[1], p[2], n[0], n[1], n[2], 0.0, 0.0]);
            }
            bi.extend_from_slice(&[base, base + 1, base + 2, base + 2, base + 1, base + 3]);
        }
        let bvao = gl.create_vertex_array()?;
        gl.bind_vertex_array(Some(bvao));
        let bvbo = gl.create_buffer()?;
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(bvbo));
        gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes(&b), glow::STATIC_DRAW);
        Self::mesh_attributes(gl);
        let bebo = gl.create_buffer()?;
        gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(bebo));
        gl.buffer_data_u8_slice(glow::ELEMENT_ARRAY_BUFFER, bytes(&bi), glow::STATIC_DRAW);
        self.box_vao = Some(bvao);
        self.box_vbo = Some(bvbo);
        self.box_ebo = Some(bebo);
        self.box_instances = Some(gl.create_buffer()?);
        self.impostor_instances = Some(gl.create_buffer()?);

        // Fallbacks so shaders always have valid inputs.
        let pal = gl.create_texture()?;
        gl.bind_texture(glow::TEXTURE_2D, Some(pal));
        gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
        gl.tex_image_2d(glow::TEXTURE_2D, 0, glow::RGB8 as i32, 256, 2, 0, glow::RGB, glow::UNSIGNED_BYTE, glow::PixelUnpackData::Slice(Some(&vec![128u8; 256 * 2 * 3])));
        set_filter(gl, glow::TEXTURE_2D, glow::NEAREST, glow::NEAREST, glow::CLAMP_TO_EDGE);
        self.palette = Some(pal);
        let layers = gl.create_texture()?;
        gl.bind_texture(glow::TEXTURE_2D_ARRAY, Some(layers));
        gl.tex_image_3d(glow::TEXTURE_2D_ARRAY, 0, glow::RGBA8 as i32, 512, 512, 64, 0, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelUnpackData::Slice(None));
        gl.generate_mipmap(glow::TEXTURE_2D_ARRAY);
        set_filter(gl, glow::TEXTURE_2D_ARRAY, glow::LINEAR_MIPMAP_LINEAR, glow::LINEAR, glow::REPEAT);
        self.layers = Some(layers);
        let imp = gl.create_texture()?;
        gl.bind_texture(glow::TEXTURE_2D_ARRAY, Some(imp));
        gl.tex_image_3d(glow::TEXTURE_2D_ARRAY, 0, glow::RGBA8 as i32, IMPOSTOR_SIZE, IMPOSTOR_SIZE, IMPOSTOR_LAYERS, 0, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelUnpackData::Slice(None));
        gl.generate_mipmap(glow::TEXTURE_2D_ARRAY);
        set_filter(gl, glow::TEXTURE_2D_ARRAY, glow::LINEAR_MIPMAP_LINEAR, glow::LINEAR, glow::CLAMP_TO_EDGE);
        self.impostors = Some(imp);
        Ok(())
    }

    unsafe fn mesh_attributes(gl: &glow::Context) {
        gl.enable_vertex_attrib_array(0);
        gl.vertex_attrib_pointer_f32(0, 3, glow::FLOAT, false, 32, 0);
        gl.enable_vertex_attrib_array(1);
        gl.vertex_attrib_pointer_f32(1, 3, glow::FLOAT, false, 32, 12);
        gl.enable_vertex_attrib_array(2);
        gl.vertex_attrib_pointer_f32(2, 2, glow::FLOAT, false, 32, 24);
    }

    unsafe fn bind_mesh_instances(gl: &glow::Context, buffer: glow::Buffer, first: u32) {
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(buffer));
        for k in 0..4u32 {
            gl.enable_vertex_attrib_array(3 + k);
            gl.vertex_attrib_pointer_f32(3 + k, 4, glow::FLOAT, false, 64, (first * 64 + k * 16) as i32);
            gl.vertex_attrib_divisor(3 + k, 1);
        }
    }

    unsafe fn process_uploads(&mut self, gl: &glow::Context) -> Result<(), String> {
        gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
        let mut meshes_this_frame = 0;
        let mut rest = Vec::new();
        for u in std::mem::take(&mut self.uploads) {
            match u {
                Upload::Reset { cols, rows, cell, data } => {
                    if let Some(t) = self.height.take() {
                        gl.delete_texture(t);
                    }
                    let t = gl.create_texture()?;
                    gl.bind_texture(glow::TEXTURE_2D, Some(t));
                    gl.tex_image_2d(glow::TEXTURE_2D, 0, glow::R32F as i32, cols, rows, 0, glow::RED, glow::FLOAT, glow::PixelUnpackData::Slice(Some(bytes(&data))));
                    set_filter(gl, glow::TEXTURE_2D, glow::LINEAR, glow::LINEAR, glow::CLAMP_TO_EDGE);
                    self.height = Some(t);
                    self.size = [cols as f32, rows as f32];
                    self.cell = cell;
                }
                Upload::Height { x, y, w, h, data } => {
                    if let Some(t) = self.height {
                        gl.bind_texture(glow::TEXTURE_2D, Some(t));
                        gl.tex_sub_image_2d(glow::TEXTURE_2D, 0, x, y, w, h, glow::RED, glow::FLOAT, glow::PixelUnpackData::Slice(Some(bytes(&data))));
                    }
                }
                Upload::Mask { width, height, data } => {
                    if let Some(t) = self.mask.take() {
                        gl.delete_texture(t);
                    }
                    let t = gl.create_texture()?;
                    gl.bind_texture(glow::TEXTURE_2D, Some(t));
                    gl.tex_image_2d(glow::TEXTURE_2D, 0, glow::R8UI as i32, width, height, 0, glow::RED_INTEGER, glow::UNSIGNED_BYTE, glow::PixelUnpackData::Slice(Some(&data)));
                    set_filter(gl, glow::TEXTURE_2D, glow::NEAREST, glow::NEAREST, glow::CLAMP_TO_EDGE);
                    self.mask = Some(t);
                    self.mask_size = [width as f32, height as f32];
                }
                Upload::MaskRect { x, y, w, h, data } => {
                    if let Some(t) = self.mask {
                        gl.bind_texture(glow::TEXTURE_2D, Some(t));
                        gl.tex_sub_image_2d(glow::TEXTURE_2D, 0, x, y, w, h, glow::RED_INTEGER, glow::UNSIGNED_BYTE, glow::PixelUnpackData::Slice(Some(&data)));
                    }
                }
                Upload::Satellite { width, height, rgb } => {
                    if let Some(t) = self.sat.take() {
                        gl.delete_texture(t);
                    }
                    let t = gl.create_texture()?;
                    gl.bind_texture(glow::TEXTURE_2D, Some(t));
                    gl.tex_image_2d(glow::TEXTURE_2D, 0, glow::RGB8 as i32, width, height, 0, glow::RGB, glow::UNSIGNED_BYTE, glow::PixelUnpackData::Slice(Some(&rgb)));
                    gl.generate_mipmap(glow::TEXTURE_2D);
                    set_filter(gl, glow::TEXTURE_2D, glow::LINEAR_MIPMAP_LINEAR, glow::LINEAR, glow::CLAMP_TO_EDGE);
                    self.sat = Some(t);
                    self.has_sat = true;
                }
                Upload::NoSatellite => {
                    self.has_sat = false;
                }
                Upload::Detail { width, height, rgb, rect } => {
                    let t = match self.detail {
                        Some(t) => t,
                        None => {
                            let t = gl.create_texture()?;
                            self.detail = Some(t);
                            t
                        }
                    };
                    gl.bind_texture(glow::TEXTURE_2D, Some(t));
                    gl.tex_image_2d(glow::TEXTURE_2D, 0, glow::RGB8 as i32, width, height, 0, glow::RGB, glow::UNSIGNED_BYTE, glow::PixelUnpackData::Slice(Some(&rgb)));
                    gl.generate_mipmap(glow::TEXTURE_2D);
                    set_filter(gl, glow::TEXTURE_2D, glow::LINEAR_MIPMAP_LINEAR, glow::LINEAR, glow::CLAMP_TO_EDGE);
                    self.detail_rect = rect;
                }
                Upload::Palette { colors, averages } => {
                    let mut d = vec![128u8; 256 * 2 * 3];
                    for (i, c) in colors.iter().take(256).enumerate() {
                        d[i * 3..i * 3 + 3].copy_from_slice(c);
                    }
                    for (i, c) in averages.iter().take(256).enumerate() {
                        d[(256 + i) * 3..(256 + i) * 3 + 3].copy_from_slice(c);
                    }
                    gl.bind_texture(glow::TEXTURE_2D, self.palette);
                    gl.tex_sub_image_2d(glow::TEXTURE_2D, 0, 0, 0, 256, 2, glow::RGB, glow::UNSIGNED_BYTE, glow::PixelUnpackData::Slice(Some(&d)));
                }
                Upload::Layer { index, size, rgba } => {
                    if index < 64 && size == 512 {
                        gl.bind_texture(glow::TEXTURE_2D_ARRAY, self.layers);
                        gl.tex_sub_image_3d(glow::TEXTURE_2D_ARRAY, 0, 0, 0, index, 512, 512, 1, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelUnpackData::Slice(Some(&rgba)));
                        gl.generate_mipmap(glow::TEXTURE_2D_ARRAY);
                    }
                }
                Upload::ClearModels => {
                    for (_, m) in self.models.drain() {
                        gl.delete_vertex_array(m.vao);
                        gl.delete_buffer(m.vbo);
                        gl.delete_buffer(m.ebo);
                        for t in m.textures {
                            gl.delete_texture(t);
                        }
                    }
                    self.impostor_layers.clear();
                    self.next_impostor = 0;
                }
                Upload::Mesh(m) => {
                    // A few per frame keep the interface smooth while models stream in.
                    if meshes_this_frame >= 6 {
                        rest.push(Upload::Mesh(m));
                        continue;
                    }
                    meshes_this_frame += 1;
                    self.upload_mesh(gl, *m)?;
                }
            }
        }
        self.uploads.extend(rest);
        Ok(())
    }

    unsafe fn upload_mesh(&mut self, gl: &glow::Context, m: MeshData) -> Result<(), String> {
        if let Some(levels) = &m.impostor {
            if self.next_impostor < IMPOSTOR_LAYERS && !self.impostor_layers.contains_key(&m.key.0) {
                let layer = self.next_impostor;
                self.next_impostor += 1;
                gl.bind_texture(glow::TEXTURE_2D_ARRAY, self.impostors);
                for (level, (side, rgba)) in levels.iter().enumerate() {
                    gl.tex_sub_image_3d(glow::TEXTURE_2D_ARRAY, level as i32, 0, 0, layer, *side as i32, *side as i32, 1, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelUnpackData::Slice(Some(rgba)));
                }
                self.impostor_layers.insert(m.key.0, layer);
            }
        }
        if m.indices.is_empty() {
            return Ok(());
        }
        let vao = gl.create_vertex_array()?;
        gl.bind_vertex_array(Some(vao));
        let vbo = gl.create_buffer()?;
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
        gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes(&m.vertices), glow::STATIC_DRAW);
        Self::mesh_attributes(gl);
        let ebo = gl.create_buffer()?;
        gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(ebo));
        gl.buffer_data_u8_slice(glow::ELEMENT_ARRAY_BUFFER, bytes(&m.indices), glow::STATIC_DRAW);
        let mut textures = Vec::new();
        for (w, h, rgba) in &m.textures {
            let t = gl.create_texture()?;
            gl.bind_texture(glow::TEXTURE_2D, Some(t));
            gl.tex_image_2d(glow::TEXTURE_2D, 0, glow::RGBA8 as i32, *w as i32, *h as i32, 0, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelUnpackData::Slice(Some(rgba)));
            gl.generate_mipmap(glow::TEXTURE_2D);
            set_filter(gl, glow::TEXTURE_2D, glow::LINEAR_MIPMAP_LINEAR, glow::LINEAR, glow::REPEAT);
            textures.push(t);
        }
        let batches = m.batches.iter().map(|(f, c, t)| (*f, *c, t.and_then(|i| textures.get(i).copied()))).collect();
        let triangles = m.indices.len() as u32 / 3;
        self.models.insert(m.key, Model { vao, vbo, ebo, batches, textures, triangles });
        Ok(())
    }

    /// Draws one view; called inside the egui paint callback.
    pub unsafe fn paint(&mut self, gl: &glow::Context, f: &Frame) {
        if let Err(e) = self.init(gl).and_then(|_| self.process_uploads(gl)) {
            self.error = Some(e);
            return;
        }
        let Some(p) = &self.programs else { return };
        let (Some(height), Some(mask)) = (self.height, self.mask) else { return };
        let (terrain, water, mesh_prog, impostor_prog, sky) = (p.terrain, p.water, p.mesh, p.impostor, p.sky);
        let mut stats = Stats { models: self.models.len() as u32, ..Default::default() };

        gl.enable(glow::DEPTH_TEST);
        gl.depth_func(glow::LEQUAL);
        gl.depth_mask(true);
        gl.clear_depth_f32(1.0);
        gl.clear_color(0.60, 0.70, 0.82, 1.0);
        gl.clear(glow::DEPTH_BUFFER_BIT | glow::COLOR_BUFFER_BIT);
        gl.disable(glow::CULL_FACE);
        gl.disable(glow::BLEND);

        let u = |prog: glow::Program, n: &str| gl.get_uniform_location(prog, n);
        if !f.view.ortho {
            gl.depth_mask(false);
            gl.disable(glow::DEPTH_TEST);
            gl.use_program(Some(sky));
            let v = &f.view;
            gl.uniform_3_f32_slice(u(sky, "u_f").as_ref(), &v.forward);
            gl.uniform_3_f32_slice(u(sky, "u_r").as_ref(), &v.right);
            gl.uniform_3_f32_slice(u(sky, "u_u").as_ref(), &v.up);
            gl.uniform_2_f32(u(sky, "u_t").as_ref(), v.tan_half_fov * v.aspect, v.tan_half_fov);
            gl.bind_vertex_array(self.quad_vao);
            gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
            gl.enable(glow::DEPTH_TEST);
            gl.depth_mask(true);
        }
        // Terrain.
        gl.use_program(Some(terrain));
        gl.uniform_matrix_4_f32_slice(u(terrain, "u_vp").as_ref(), false, &f.view.vp);
        gl.uniform_2_f32(u(terrain, "u_size").as_ref(), self.size[0], self.size[1]);
        gl.uniform_1_f32(u(terrain, "u_cell").as_ref(), self.cell);
        gl.uniform_1_f32(u(terrain, "u_world").as_ref(), f.world);
        gl.uniform_3_f32(u(terrain, "u_eye").as_ref(), f.view.eye[0], f.view.eye[1], f.view.eye[2]);
        gl.uniform_1_f32_slice(u(terrain, "u_range").as_ref(), &f.ranges);
        let mode = Shading::ALL.iter().position(|s| *s == f.shading).unwrap_or(0) as i32;
        gl.uniform_1_i32(u(terrain, "u_mode").as_ref(), mode);
        gl.uniform_1_i32(u(terrain, "u_ortho").as_ref(), f.view.ortho as i32);
        gl.uniform_1_i32(u(terrain, "u_has_sat").as_ref(), self.has_sat as i32);
        gl.uniform_4_f32_slice(u(terrain, "u_detail_rect").as_ref(), &self.detail_rect);
        gl.uniform_2_f32(u(terrain, "u_mask_size").as_ref(), self.mask_size[0], self.mask_size[1]);
        gl.uniform_1_f32(u(terrain, "u_tile").as_ref(), f.tile);
        gl.uniform_1_f32(u(terrain, "u_sea").as_ref(), f.sea);
        gl.uniform_2_f32(u(terrain, "u_height_range").as_ref(), f.height_range[0], f.height_range[1]);
        gl.uniform_4_f32(u(terrain, "u_grid").as_ref(), f.grid[0], f.grid[1], f.grid[2], 0.0);
        match f.brush {
            Some(b) => {
                gl.uniform_4_f32(u(terrain, "u_brush").as_ref(), b.x, b.z, b.radius, b.falloff);
                gl.uniform_4_f32(u(terrain, "u_brush_color").as_ref(), b.color[0], b.color[1], b.color[2], 1.0);
            }
            None => gl.uniform_4_f32(u(terrain, "u_brush").as_ref(), 0.0, 0.0, 0.0, 0.0),
        }
        let bind = |unit: u32, target: u32, tex: Option<glow::Texture>, name: &str| {
            gl.active_texture(glow::TEXTURE0 + unit);
            gl.bind_texture(target, tex);
            gl.uniform_1_i32(u(terrain, name).as_ref(), unit as i32);
        };
        bind(0, glow::TEXTURE_2D, Some(height), "u_height");
        bind(1, glow::TEXTURE_2D, Some(mask), "u_mask");
        bind(2, glow::TEXTURE_2D, self.sat.or(self.palette), "u_sat");
        bind(3, glow::TEXTURE_2D, self.detail.or(self.palette), "u_detail");
        bind(4, glow::TEXTURE_2D, self.palette, "u_palette");
        bind(5, glow::TEXTURE_2D_ARRAY, self.layers, "u_layers");
        gl.bind_vertex_array(self.patch_vao);
        gl.bind_buffer(glow::ARRAY_BUFFER, self.patch_instances);
        gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes(&f.patches), glow::STREAM_DRAW);
        if f.wireframe {
            gl.polygon_mode(glow::FRONT_AND_BACK, glow::LINE);
        }
        gl.draw_elements_instanced(glow::TRIANGLES, self.index_count, glow::UNSIGNED_INT, 0, f.patches.len() as i32);
        if f.wireframe {
            gl.polygon_mode(glow::FRONT_AND_BACK, glow::FILL);
        }
        stats.patches = f.patches.len() as u32;
        stats.triangles += (f.patches.len() as u64) * (self.index_count as u64 / 3);
        stats.draw_calls += 1;

        // Objects.
        gl.active_texture(glow::TEXTURE0);
        gl.use_program(Some(mesh_prog));
        gl.uniform_matrix_4_f32_slice(u(mesh_prog, "u_vp").as_ref(), false, &f.view.vp);
        gl.uniform_3_f32(u(mesh_prog, "u_eye").as_ref(), f.view.eye[0], f.view.eye[1], f.view.eye[2]);
        gl.uniform_1_i32(u(mesh_prog, "u_tex").as_ref(), 0);
        gl.uniform_1_f32(u(mesh_prog, "u_alpha").as_ref(), 1.0);
        let textured = u(mesh_prog, "u_textured");
        if !f.objects.groups.is_empty() {
            let buffer = match self.mesh_instances.get(&f.viewport_key) {
                Some(b) => *b,
                None => {
                    let b = gl.create_buffer().unwrap();
                    self.mesh_instances.insert(f.viewport_key, b);
                    b
                }
            };
            let ptr = Arc::as_ptr(&f.objects.instances);
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(buffer));
            if self.instances_uploaded.get(&f.viewport_key) != Some(&ptr) {
                gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes(&f.objects.instances), glow::STREAM_DRAW);
                self.instances_uploaded.insert(f.viewport_key, ptr);
            }
            for (key, first, count) in f.objects.groups.iter() {
                let Some(m) = self.models.get(key) else { continue };
                gl.bind_vertex_array(Some(m.vao));
                Self::bind_mesh_instances(gl, buffer, *first);
                for (start, n, tex) in &m.batches {
                    gl.bind_texture(glow::TEXTURE_2D, *tex);
                    gl.uniform_1_i32(textured.as_ref(), tex.is_some() as i32);
                    gl.draw_elements_instanced(glow::TRIANGLES, *n as i32, glow::UNSIGNED_INT, (*start * 4) as i32, *count as i32);
                    stats.draw_calls += 1;
                }
                stats.triangles += m.triangles as u64 * *count as u64;
                stats.mesh_instances += *count;
            }
        }
        if !f.objects.boxes.is_empty() {
            gl.bind_vertex_array(self.box_vao);
            Self::bind_mesh_instances(gl, self.box_instances.unwrap(), 0);
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes(&f.objects.boxes), glow::STREAM_DRAW);
            gl.uniform_1_i32(textured.as_ref(), 0);
            if f.view.ortho {
                gl.enable(glow::BLEND);
                gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
                gl.uniform_1_f32(u(mesh_prog, "u_alpha").as_ref(), 0.55);
            }
            gl.draw_elements_instanced(glow::TRIANGLES, 36, glow::UNSIGNED_INT, 0, (f.objects.boxes.len() / 16) as i32);
            gl.disable(glow::BLEND);
            gl.uniform_1_f32(u(mesh_prog, "u_alpha").as_ref(), 1.0);
            stats.draw_calls += 1;
        }
        if !f.objects.impostors.is_empty() {
            gl.use_program(Some(impostor_prog));
            gl.uniform_matrix_4_f32_slice(u(impostor_prog, "u_vp").as_ref(), false, &f.view.vp);
            gl.uniform_3_f32(u(impostor_prog, "u_right").as_ref(), f.view.right[0], f.view.right[1], f.view.right[2]);
            gl.uniform_3_f32(u(impostor_prog, "u_eye").as_ref(), f.view.eye[0], f.view.eye[1], f.view.eye[2]);
            gl.uniform_1_i32(u(impostor_prog, "u_tex").as_ref(), 0);
            gl.bind_texture(glow::TEXTURE_2D_ARRAY, self.impostors);
            gl.bind_vertex_array(self.quad_vao);
            gl.bind_buffer(glow::ARRAY_BUFFER, self.impostor_instances);
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes(&f.objects.impostors), glow::STREAM_DRAW);
            gl.enable_vertex_attrib_array(1);
            gl.vertex_attrib_pointer_f32(1, 4, glow::FLOAT, false, 32, 0);
            gl.vertex_attrib_divisor(1, 1);
            gl.enable_vertex_attrib_array(2);
            gl.vertex_attrib_pointer_f32(2, 4, glow::FLOAT, false, 32, 16);
            gl.vertex_attrib_divisor(2, 1);
            gl.draw_arrays_instanced(glow::TRIANGLE_STRIP, 0, 4, (f.objects.impostors.len() / 8) as i32);
            // The quad VAO also draws water, without instancing.
            gl.vertex_attrib_divisor(1, 0);
            gl.vertex_attrib_divisor(2, 0);
            gl.disable_vertex_attrib_array(1);
            gl.disable_vertex_attrib_array(2);
            stats.triangles += (f.objects.impostors.len() / 8) as u64 * 2;
            stats.impostors = (f.objects.impostors.len() / 8) as u32;
            stats.draw_calls += 1;
        }

        if f.show_water && !matches!(f.shading, Shading::Mask | Shading::Height | Shading::Slope) {
            gl.enable(glow::BLEND);
            gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
            gl.depth_mask(false);
            gl.use_program(Some(water));
            gl.uniform_matrix_4_f32_slice(u(water, "u_vp").as_ref(), false, &f.view.vp);
            gl.uniform_3_f32(u(water, "u_eye").as_ref(), f.view.eye[0], f.view.eye[1], f.view.eye[2]);
            gl.uniform_1_f32(u(water, "u_sea").as_ref(), f.sea);
            let m = f.world * 3.0;
            gl.uniform_4_f32(u(water, "u_area").as_ref(), -m, -m, f.world + 2.0 * m, f.world + 2.0 * m);
            gl.bind_vertex_array(self.quad_vao);
            gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
            gl.depth_mask(true);
        }
        gl.disable(glow::DEPTH_TEST);
        gl.enable(glow::BLEND);
        gl.bind_vertex_array(None);
        gl.use_program(None);
        self.stats = stats;
    }
}

pub fn callback(r: Arc<std::sync::Mutex<Renderer>>, frame: Frame) -> eframe::egui::PaintCallback {
    let rect = frame.view.rect;
    eframe::egui::PaintCallback {
        rect,
        callback: Arc::new(egui_glow::CallbackFn::new(move |_info, painter| {
            if let Ok(mut r) = r.lock() {
                unsafe { r.paint(painter.gl(), &frame) };
            }
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::Camera;
    use eframe::egui::{pos2, vec2};

    fn hm() -> Heightmap {
        let mut h = Heightmap::new(2048, 2048, 6.0, 50.0);
        h.set(100, 100, 300.0);
        h
    }

    fn view(y: f32) -> View {
        let cam = Camera { pos: [6000.0, y, 6000.0], yaw: 0.3, pitch: -0.3, fov: 1.2, speed: 1.0 };
        View::perspective(&cam, eframe::egui::Rect::from_min_size(pos2(0.0, 0.0), vec2(1600.0, 900.0)), 30.0)
    }

    #[test]
    fn pyramid_tracks_extremes_and_updates() {
        let mut h = hm();
        let mut l = TerrainLod::build(&h);
        assert_eq!(l.levels.last().unwrap()[0], [50.0, 300.0]);
        h.set(100, 100, 900.0);
        l.update(&h, Rect { x0: 100, y0: 100, x1: 100, y1: 100 });
        assert_eq!(l.levels.last().unwrap()[0], [50.0, 900.0]);
    }

    #[test]
    fn selection_is_bounded_and_finest_near_the_camera() {
        let l = TerrainLod::build(&hm());
        let v = view(100.0);
        let p = l.select(&v, 1.0);
        assert!(p.len() > 8 && p.len() < 1200, "{} patches", p.len());
        assert!(p.iter().any(|q| q[3] == 0.0));
        assert!(p.iter().any(|q| q[3] >= 3.0));
        // Quality only trades detail, patches never overlap on a level.
        let mut seen = std::collections::HashSet::new();
        for q in &p {
            assert!(seen.insert((q[0] as i32, q[1] as i32, q[3] as i32)));
        }
    }

    #[test]
    fn high_camera_uses_coarse_patches() {
        let l = TerrainLod::build(&hm());
        let far = l.select(&view(9000.0), 1.0);
        assert!(far.iter().all(|q| q[3] >= 1.0));
    }
}
