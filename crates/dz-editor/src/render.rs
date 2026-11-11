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

/// Sky reflection and water surface, shared by the sea and by river and pond models.
const WATER_GLSL: &str = r#"
uniform float u_time;
vec3 environment(vec3 d){
  float day = smoothstep(-0.12, 0.3, u_sun.y);
  if (d.y < 0.0) return mix(vec3(0.03,0.04,0.05), vec3(0.22,0.24,0.18), day);
  vec3 sky = mix(vec3(0.73,0.79,0.86), vec3(0.38,0.53,0.74), pow(d.y, 0.55));
  return mix(vec3(0.09,0.11,0.2), sky, day);
}
vec2 ripple(vec2 p, vec2 dir, float f, float a, float s){
  return dir * f * a * cos(dot(dir, p) * f + u_time * s);
}
vec4 shadeWater(vec3 wp, float fog, vec2 swell, float foam){
  float dist = distance(wp, u_eye);
  vec3 V = normalize(u_eye - wp);
  vec2 g = ripple(wp.xz, vec2(0.8, 0.6), 0.9, 0.035, 1.3)
         + ripple(wp.xz, vec2(-0.5, 0.86), 1.7, 0.02, 1.9)
         + ripple(wp.xz, vec2(0.28, -0.96), 3.6, 0.009, 2.8)
         + ripple(wp.xz, vec2(-0.93, -0.36), 7.1, 0.004, 4.1);
  // Ripples blur into a calm surface with distance.
  g *= 1.0 / (1.0 + dist * 0.012);
  g += swell;
  vec3 N = normalize(vec3(-g.x, 1.0, -g.y));
  float f = 0.02 + 0.98 * pow(1.0 - max(dot(N, V), 0.0), 5.0);
  vec3 deep = vec3(0.025, 0.11, 0.14) * (u_amb * 1.1 + u_sun_col * max(u_sun.y, 0.0) * 1.2);
  vec3 r = reflect(-V, N);
  // At grazing angles a wave can reflect below the horizon; the sea still mirrors the sky.
  vec3 col = mix(deep, environment(vec3(r.x, abs(r.y), r.z)), f);
  vec3 H = normalize(V + u_sun);
  col += u_sun_col * pow(max(dot(N, H), 0.0), 380.0) * 1.8 * step(0.0, u_sun.y);
  col = mix(col, (u_amb + u_sun_col * max(u_sun.y, 0.0)) * 0.9, foam);
  col = mix(col, vec3(0.73,0.79,0.86), 1.0 - exp(-dist * fog));
  return vec4(col * u_exposure, max(mix(0.74, 0.96, f), foam));
}
"#;

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
uniform usampler2D u_holes;
uniform int u_has_holes;
uniform vec2 u_mask_size;
uniform sampler2DArray u_layers, u_layer_normals;
uniform float u_tile;
uniform vec4 u_brush;
uniform vec4 u_brush_color;
uniform vec4 u_grid;
uniform float u_sea;
uniform vec2 u_height_range;
uniform int u_ortho;
uniform sampler2DShadow u_shadow;
uniform mat4 u_light;
uniform int u_has_shadow;
uniform vec3 u_sun, u_sun_col, u_amb;
uniform float u_fog, u_exposure, u_bias, u_transition, u_surface_range;
uniform int u_brush_shape;
vec3 tone(vec3 c){
  // Linear up to 0.7, then a soft shoulder so bright surfaces keep their detail.
  vec3 over = max(c - vec3(0.7), vec3(0.0));
  return min(c, vec3(0.7)) + 0.3 * (vec3(1.0) - exp(-over / 0.3));
}
const vec3 FOG = vec3(0.73, 0.79, 0.86);

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
  if (u_has_holes != 0 && texelFetch(u_holes, ivec2(floor(v_world.xz / u_cell)), 0).r > 0u) discard;
  vec2 uv = vec2(v_world.x, u_world - v_world.z) / u_world;
  vec3 n = normalAt(v_world.xz);
  float dist = distance(v_world, u_eye);
  vec3 col;
  // Surface blending: four nearest mask pixels.
  vec2 mp = uv * u_mask_size - 0.5;
  int span = 1;
  if (u_mode == 0 && u_transition > 0.0) {
    // Surfaces meet over a few metres along an uneven edge, as they do in the game.
    float perM = u_mask_size.x / u_world;
    vec2 w = v_world.xz;
    vec2 wob = vec2(sin(w.x*0.63+1.7*sin(w.y*0.37)) + 0.5*sin(w.y*1.91+w.x*0.7), sin(w.y*0.71+1.3*sin(w.x*0.43)) + 0.5*sin(w.x*2.13-w.y*0.6));
    mp += wob * u_transition * perM * 0.3;
    span = max(1, int(u_transition * perM + 0.5));
  }
  ivec2 ip = ivec2(floor(mp / float(span))) * span;
  vec2 f = (mp - vec2(ip)) / float(span);
  uint a = maskAt(ip), b = maskAt(ip + ivec2(span,0)), c = maskAt(ip + ivec2(0,span)), d = maskAt(ip + ivec2(span,span));
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
  float shade = 1.0;
  if (u_has_shadow != 0 && u_mode == 0) {
    vec4 q = u_light * vec4(v_world, 1.0);
    vec3 p = q.xyz * 0.5 + 0.5;
    vec2 edge = min(p.xy, 1.0 - p.xy);
    if (min(edge.x, edge.y) > 0.0) {
      float t = 1.0 / 4096.0, s = 0.0;
      for (int i = 0; i < 4; i++) {
        vec2 o = vec2((i & 1) * 2 - 1, (i & 2) - 1) * t;
        s += texture(u_shadow, vec3(p.xy + o, p.z - u_bias));
      }
      shade = mix(1.0, s * 0.25, smoothstep(0.0, 0.08, min(edge.x, edge.y)));
    }
  }
  vec3 light = u_amb * (0.6 + 0.4*n.y) + u_sun_col * max(dot(n, u_sun), 0.0) * shade;

  if (u_mode == 0) { // lit, like in the game
    vec3 alb = sat;
    vec3 bn = n;
    float near = 1.0 - smoothstep(u_ortho != 0 ? 0.0 : u_surface_range * 0.21, u_ortho != 0 ? u_surface_range * 3.7 : u_surface_range, dist);
    if (near > 0.002) {
      vec2 t = v_world.xz * u_tile;
      vec2 dx = dFdx(t), dy = dFdy(t);
      vec4 ra = textureGrad(u_layers, vec3(t, float(a)), dx, dy), rb = textureGrad(u_layers, vec3(t, float(b)), dx, dy);
      vec4 rc = textureGrad(u_layers, vec3(t, float(c)), dx, dy), rd = textureGrad(u_layers, vec3(t, float(d)), dx, dy);
      vec3 ta = ra.rgb, tb = rb.rgb, tc = rc.rgb, td = rd.rgb;
      // Taller parts of a surface (its alpha) show through first where two meet.
      vec4 lift = vec4(wa, wb, wc, wd) * (vec4(ra.a, rb.a, rc.a, rd.a) * 0.75 + 0.25);
      lift = pow(lift, vec4(3.5));
      lift /= max(lift.x + lift.y + lift.z + lift.w, 1e-4);
      wa = lift.x; wb = lift.y; wc = lift.z; wd = lift.w;
      vec3 s = ta / max(avg(a), vec3(0.05)) * wa + tb / max(avg(b), vec3(0.05)) * wb + tc / max(avg(c), vec3(0.05)) * wc + td / max(avg(d), vec3(0.05)) * wd;
      vec3 raw = ta * wa + tb * wb + tc * wc + td * wd;
      // Close up the game shows the surface texture itself, tinted a little by the satellite.
      vec3 tinted = sat * clamp(s, vec3(0.4), vec3(1.6));
      alb = mix(sat, mix(raw, tinted, u_has_sat != 0 ? 0.45 : 0.0), near);
      // Normal maps of the surfaces; their texture runs east and south over the ground.
      vec3 tn = (textureGrad(u_layer_normals, vec3(t, float(a)), dx, dy).xyz * wa + textureGrad(u_layer_normals, vec3(t, float(b)), dx, dy).xyz * wb
               + textureGrad(u_layer_normals, vec3(t, float(c)), dx, dy).xyz * wc + textureGrad(u_layer_normals, vec3(t, float(d)), dx, dy).xyz * wd) * 2.0 - 1.0;
      bn = normalize(n + vec3(tn.x, 0.0, tn.y) * near);
    }
    // Light adds up in linear space, as it does in the game.
    vec3 sky = u_amb * (0.6 + 0.4*n.y);
    vec3 A = pow(sky, vec3(2.2));
    vec3 S = pow(sky + u_sun_col, vec3(2.2)) - A;
    col = pow(pow(alb, vec3(2.2)) * (A + S * max(dot(bn, u_sun), 0.0) * shade), vec3(1.0 / 2.2));
  } else if (u_mode == 1) { // satellite
    col = sat;
  } else if (u_mode == 2) { // mask, hard edges
    col = pal(maskAt(ivec2(floor(uv * u_mask_size))));
  } else if (u_mode == 3) { // hillshade
    col = vec3(0.62) * light;
  } else if (u_mode == 4) { // height
    float t = (v_world.y - u_height_range.x) / max(u_height_range.y - u_height_range.x, 1.0);
    col = ramp(t) * (0.45 + 0.55 * max(dot(n, u_sun), 0.0) / 0.8);
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
    // Lines too close together on screen would only darken the ground.
    col = mix(col, vec3(0.2,0.1,0.05), (1.0 - clamp(g, 0.0, 1.0)) * 0.45 * (1.0 - smoothstep(0.15, 0.5, fwidth(l))));
    float major = abs(fract(l / 5.0 - 0.5) - 0.5) / fwidth(l / 5.0);
    col = mix(col, vec3(0.16,0.07,0.03), (1.0 - clamp(major * 0.6, 0.0, 1.0)) * 0.6 * (1.0 - smoothstep(0.15, 0.5, fwidth(l / 5.0))));
  }
  if (u_brush.z > 0.0) {
    vec2 bv = abs(v_world.xz - u_brush.xy);
    float bd = u_brush_shape == 1 ? max(bv.x, bv.y) : (u_brush_shape == 2 ? bv.x + bv.y : length(bv));
    float px = fwidth(bd) * 1.5;
    float ring = 1.0 - clamp(abs(bd - u_brush.z) / px, 0.0, 1.0);
    float inner = 1.0 - clamp(abs(bd - u_brush.z * (1.0 - u_brush.w)) / px, 0.0, 1.0);
    float fill = bd < u_brush.z ? 0.10 : 0.0;
    col = mix(col, u_brush_color.rgb, max(max(ring, inner * 0.5), fill));
  }
  if (u_ortho == 0 && u_mode <= 1) col = mix(col, FOG, 1.0 - exp(-dist * u_fog));
  if (u_mode <= 1) col = tone(col * u_exposure);
  o = vec4(col, 1.0);
}"#;

const WATER_VS: &str = r#"#version 330 core
layout(location=0) in vec2 a_pos;
uniform mat4 u_vp; uniform vec4 u_area; uniform float u_sea, u_time, u_waves, u_cell;
uniform vec3 u_eye; uniform vec2 u_size; uniform sampler2D u_height;
out vec3 v_world; out vec2 v_swell; out float v_depth;
// (direction, wave length, relative height)
const vec4 W[5] = vec4[5](vec4(0.83,0.55,92.0,1.0), vec4(0.62,0.78,57.0,0.55), vec4(0.97,0.22,38.0,0.3), vec4(0.34,0.94,23.0,0.16), vec4(0.9,-0.43,14.0,0.08));
void main(){
  // Grid packed round the camera, thinning out to the horizon.
  vec2 q = a_pos * 2.0 - 1.0;
  vec2 p = u_eye.xz + q * abs(q) * abs(q) * u_area.z;
  vec2 uv = (p / u_cell + 0.5) / u_size;
  bool on = uv.x > 0.0 && uv.y > 0.0 && uv.x < 1.0 && uv.y < 1.0;
  v_depth = on ? u_sea - texture(u_height, uv).r : 100.0;
  // Swell dies in shallow water and is not worth drawing far away.
  float shore = clamp(v_depth / 7.0, 0.12, 1.0);
  float amp = u_waves * shore * (1.0 - smoothstep(900.0, 2600.0, distance(p, u_eye.xz)));
  float h = 0.0; vec2 slope = vec2(0.0), shift = vec2(0.0);
  for (int i = 0; i < 5; i++) {
    float k = 6.28318 / W[i].z;
    float speed = sqrt(9.81 * k);
    float phase = k * dot(W[i].xy, p) - speed * u_time;
    float a = amp * W[i].w * 0.5;
    h += a * sin(phase);
    slope += W[i].xy * k * a * cos(phase);
    shift -= W[i].xy * a * 0.6 * cos(phase);
  }
  v_swell = slope;
  v_world = vec3(p.x + shift.x, u_sea + h, p.y + shift.y);
  gl_Position = u_vp * vec4(v_world, 1.0);
  // Far away the surface and the sea bed under it would fight over the same depth.
  gl_Position.z -= gl_Position.w * 0.0004 * smoothstep(200.0, 2500.0, distance(p, u_eye.xz));
}"#;

const WATER_FS: &str = r#"#version 330 core
in vec3 v_world; in vec2 v_swell; in float v_depth; out vec4 o;
uniform vec3 u_eye, u_sun, u_sun_col, u_amb; uniform float u_fog, u_exposure;
//WATER
void main(){
  if (v_depth < -0.3) discard;
  // Foam rolls in along the shore.
  float band = 0.5 + 0.5 * sin(v_depth * 2.6 - u_time * 0.8 + 1.4 * sin(v_world.x * 0.11 + v_world.z * 0.07));
  float lace = 0.5 + 0.5 * sin(v_world.x * 2.3 + sin(v_world.z * 1.7) * 2.0 + u_time * 0.5) * sin(v_world.z * 2.9 - u_time * 0.3);
  float foam = (1.0 - smoothstep(0.0, 1.6, v_depth)) * smoothstep(0.35, 0.95, band * 0.7 + lace * 0.45) * 0.85;
  o = shadeWater(v_world, u_fog, v_swell, foam);
}"#;

const MESH_VS: &str = r#"#version 330 core
layout(location=0) in vec3 a_pos;
layout(location=1) in vec3 a_normal;
layout(location=2) in vec2 a_uv;
layout(location=7) in vec2 a_uv2;
layout(location=3) in vec4 i_r0;
layout(location=4) in vec4 i_r1;
layout(location=5) in vec4 i_r2;
layout(location=6) in vec4 i_tint;
uniform mat4 u_vp;
uniform int u_conform;
uniform sampler2D u_height;
uniform vec2 u_size;
uniform float u_cell;
uniform vec3 u_eye;
out vec3 v_n; out vec2 v_uv; out vec2 v_uv2; out vec3 v_world; out vec3 v_round; flat out vec4 v_tint;
void main(){
  vec4 p = vec4(a_pos, 1.0);
  v_world = vec3(dot(i_r0, p), dot(i_r1, p), dot(i_r2, p));
  if (u_conform != 0) {
    float ground = texture(u_height, (v_world.xz / u_cell + 0.5) / u_size).r;
    v_world.y = ground + (v_world.y - i_r1.w) + 0.05 + distance(v_world, u_eye) * 0.0008;
  }
  vec4 n = vec4(a_normal, 0.0);
  v_n = normalize(vec3(dot(i_r0, n), dot(i_r1, n), dot(i_r2, n)));
  // A crown is lit as one rounded mass: outwards from the trunk and up.
  vec3 r = vec3(a_pos.x, 0.0, a_pos.z);
  vec4 s = vec4(normalize(vec3(r.x, length(r) * 0.55 + 0.35, r.z)), 0.0);
  v_round = vec3(dot(i_r0, s), dot(i_r1, s), dot(i_r2, s));
  v_uv = a_uv; v_uv2 = a_uv2; v_tint = i_tint;
  gl_Position = u_vp * vec4(v_world, 1.0);
}"#;

const MESH_FS: &str = r#"#version 330 core
in vec3 v_n; in vec2 v_uv; in vec2 v_uv2; in vec3 v_world; in vec3 v_round; flat in vec4 v_tint;
out vec4 o;
uniform sampler2D u_tex, u_m1, u_m2, u_m3, u_m4, u_m5, u_m6, u_m7, u_m8, u_m9, u_m10, u_m11, u_m12, u_m13, u_m14;
uniform vec4 u_xf[15];
uniform vec3 u_xo[15];
uniform int u_textured, u_mode;
uniform vec3 u_eye, u_sun, u_sun_col, u_amb;
uniform float u_alpha, u_fog, u_exposure, u_bias, u_f0;
// Material colours: emissive, ambient, diffuse, forced diffuse, specular.
uniform vec3 u_mc[5];
uniform float u_power, u_cut;
// Multi layer laid over upward faces: layer, cosine where it starts, cosine where it is full.
uniform vec3 u_top;
uniform int u_decal;
uniform sampler2DShadow u_shadow;
uniform mat4 u_light;
uniform int u_has_shadow;

// Channels a compressed texture stores upside down, per slot.
uniform vec4 u_inv[15];
vec2 UV(int i){
  vec2 b = mod(u_xo[i].z, 2.0) > 0.5 ? v_uv2 : v_uv;
  return vec2(u_xf[i].x * b.x + u_xf[i].z * b.y + u_xo[i].x, u_xf[i].y * b.x + u_xf[i].w * b.y + u_xo[i].y);
}
vec4 inv(vec4 v, int i){ return mix(v, vec4(1.0) - v, u_inv[i]); }
vec4 T(sampler2D s, int i){ return inv(texture(s, UV(i)), i); }
// Tangent frame from screen derivatives, so meshes need no stored tangents.
vec3 bump(vec3 N, vec3 m, vec2 uv){
  vec3 p1 = dFdx(v_world), p2 = dFdy(v_world);
  vec2 a = dFdx(uv), b = dFdy(uv);
  vec3 r1 = cross(p2, N), r2 = cross(N, p1);
  vec3 T = r1 * a.x + r2 * b.x;
  vec3 B = r1 * a.y + r2 * b.y;
  float k = max(dot(T, T), dot(B, B));
  if (k <= 0.0) return N;
  k = 1.0 / sqrt(k);
  return normalize(T * k * m.x + B * k * m.y + N * m.z);
}
vec3 tone(vec3 c){
  // Linear up to 0.7, then a soft shoulder so bright surfaces keep their detail.
  vec3 over = max(c - vec3(0.7), vec3(0.0));
  return min(c, vec3(0.7)) + 0.3 * (vec3(1.0) - exp(-over / 0.3));
}
// Colour textures are stored for the screen; light is added up in linear space, as in the game.
vec3 lin(vec3 c){ return pow(max(c, vec3(0.0)), vec3(2.2)); }
float shadow(vec3 n){
  if (u_has_shadow == 0) return 1.0;
  vec4 q = u_light * vec4(v_world + n * 0.15, 1.0);
  vec3 p = q.xyz * 0.5 + 0.5;
  vec2 edge = min(p.xy, 1.0 - p.xy);
  if (min(edge.x, edge.y) <= 0.0) return 1.0;
  float t = 1.0 / 4096.0, s = 0.0;
  for (int i = 0; i < 4; i++) {
    vec2 d = vec2((i & 1) * 2 - 1, (i & 2) - 1) * t;
    s += texture(u_shadow, vec3(p.xy + d, p.z - u_bias * 1.25));
  }
  return mix(1.0, s * 0.25, smoothstep(0.0, 0.08, min(edge.x, edge.y)));
}
//WATER
void main(){
  if (u_mode == 3) {
    vec4 w = shadeWater(v_world, u_fog, vec2(0.0), 0.0);
    // Rivers and ponds are shallow: they mirror more of the sky and show their bed.
    vec3 up = environment(reflect(-normalize(u_eye - v_world), vec3(0.0, 1.0, 0.0)));
    o = vec4(mix(w.rgb, up * u_exposure, 0.32), min(w.a, 0.84));
    return;
  }
  float dist = distance(v_world, u_eye);
  float cut = u_decal != 0 ? 0.02 : (u_cut > 0.0 ? u_cut : mix(0.42, 0.1, clamp(dist / 350.0, 0.0, 1.0)));
  vec3 G = normalize(v_n);
  vec3 N = G;
  vec3 V = normalize(u_eye - v_world);
  vec4 c = vec4(0.62,0.62,0.6,1.0);
  float ao = 1.0, shade = 1.0, gloss = 32.0;
  vec3 spec = vec3(0.0), kE = vec3(0.0), kA = vec3(1.0), kD = vec3(1.0), kF = vec3(0.0);
  // Faces without a material and foliage are lit from both sides.
  bool lit_sides = u_mode != 0 && u_mode != 4;
  if (lit_sides && dot(N, V) < 0.0) N = -N;
  if (u_mode != 0) { kE = u_mc[0]; kA = u_mc[1]; kD = u_mc[2]; kF = u_mc[3]; }
  if (u_mode == 1) {
    c = T(u_tex, 0);
    if (c.a < cut) discard;
    // The macro map shades the colour around mid grey, as far as its alpha says: the folds of
    // a curtain, the dirt of a wall.
    vec4 mc = T(u_m3, 3);
    c.rgb = lin(c.rgb) * mix(vec3(1.0), mc.rgb * 2.0, mc.a);
    c.rgb *= T(u_m2, 2).rgb * 2.0;
    // Green of the ambient map shades the sky light, blue the sun light.
    vec4 as = T(u_m4, 4);
    ao = as.g; shade = as.b;
    vec4 s = T(u_m5, 5);
    spec = u_mc[4] * s.g;
    gloss = max(u_power * s.b, 1.0);
    N = bump(N, T(u_m1, 1).xyz * 2.0 - 1.0, UV(1));
  } else if (u_mode == 2) {
    vec4 m = T(u_m4, 4);
    float base = max(1.0 - m.r - m.g - m.b, 0.0);
    if (u_top.x >= 0.0) {
      vec4 w = vec4(base, m.rgb);
      vec4 only = vec4(equal(vec4(u_top.x), vec4(0.0, 1.0, 2.0, 3.0)));
      w = mix(w, only, smoothstep(u_top.z, u_top.y, G.y));
      base = w.x; m.rgb = w.yzw;
    }
    vec4 d0 = T(u_m11, 11), d1 = T(u_m12, 12), d2 = T(u_m13, 13), d3 = T(u_m14, 14);
    // A plain specular map in a detail slot has no detail pattern in its red.
    if (u_xo[11].z > 1.5) d0.r = 0.5;
    if (u_xo[12].z > 1.5) d1.r = 0.5;
    if (u_xo[13].z > 1.5) d2.r = 0.5;
    if (u_xo[14].z > 1.5) d3.r = 0.5;
    // Red of dtsmdi is detail around mid grey, green specular, blue gloss.
    vec4 mc = T(u_m5, 5);
    vec3 l0 = lin(T(u_tex, 0).rgb) * d0.r * 2.0;
    c.rgb = l0 * base + lin(T(u_m1, 1).rgb) * d1.r * 2.0 * m.r + lin(T(u_m2, 2).rgb) * d2.r * 2.0 * m.g + lin(T(u_m3, 3).rgb) * d3.r * 2.0 * m.b;
    // The macro map shades all the layers around mid grey, as far as its alpha says: it turns
    // plain boards dark, lifts the brick of a wall and lays rust over the paint of a car.
    c.rgb *= mix(vec3(1.0), mc.rgb * 2.0, mc.a);
    c.a = 1.0;
    spec = u_mc[4] * (d0.g * base + d1.g * m.r + d2.g * m.g + d3.g * m.b);
    gloss = max(u_power * (d0.b * base + d1.b * m.r + d2.b * m.g + d3.b * m.b), 1.0);
    vec4 as = T(u_m6, 6);
    ao = as.g; shade = as.b;
    vec3 n = (T(u_m7, 7).xyz * base + T(u_m8, 8).xyz * m.r + T(u_m9, 9).xyz * m.g + T(u_m10, 10).xyz * m.b) * 2.0 - 1.0;
    N = bump(N, n, UV(0));
  } else if (u_mode == 4) {
    c = T(u_tex, 0);
    if (c.a < cut) discard;
    // The macro map tints the crown around mid grey; its alpha is the shade inside it.
    vec4 mc = T(u_m3, 3);
    c.rgb = lin(c.rgb) * mc.rgb * 2.0;
    ao = 0.55 + 0.45 * mc.a;
    if (v_tint.a < 0.0) {
      // Grass takes the colour of the ground it stands on, as far as the setting says.
      float l = dot(c.rgb, vec3(0.3, 0.5, 0.2));
      c.rgb = mix(c.rgb, lin(v_tint.rgb) * clamp(l / 0.12, 0.5, 1.8), -v_tint.a);
      N = normalize(mix(N, vec3(0.0, 1.0, 0.0), 0.8));
    } else {
      N = normalize(mix(N, normalize(v_round), 0.3));
    }
    N = bump(N, T(u_m1, 1).xyz * 2.0 - 1.0, UV(1));
  } else if (u_mode == 6) {
    c = T(u_tex, 0);
    if (c.a < cut) discard;
    c.rgb = lin(c.rgb) * T(u_m3, 3).rgb * 2.0;
    N = bump(N, T(u_m1, 1).xyz * 2.0 - 1.0, UV(1));
  } else if (u_mode == 5) {
    c = T(u_tex, 0);
    c.rgb = lin(c.rgb);
    spec = max(u_mc[4], vec3(0.25));
    gloss = max(u_power, 48.0);
  } else if (u_textured != 0) {
    c = inv(texture(u_tex, v_uv), 0);
    if (c.a < cut) discard;
    c.rgb = lin(c.rgb);
  } else if (v_tint.a > 0.0 && v_tint.a < 0.99) {
    c.rgb = lin(v_tint.rgb);
  } else {
    c.rgb = lin(c.rgb);
  }
  float ndl = lit_sides ? max(dot(N, u_sun), 0.0) : abs(dot(N, u_sun));
  // Foliage is too sparse to shadow itself the way solid surfaces do.
  float sh = lit_sides ? shadow(N) : mix(1.0, shadow(vec3(0.0, 1.0, 0.0)), 0.3);
  if (u_mode == 4) {
    // Leaves: a soft edge between light and shade, and sun glowing through from behind.
    float d = dot(N, u_sun);
    ndl = max((d + 0.35) / 1.35, 0.0) + 0.3 * max(-d, 0.0) * (0.4 + 0.6 * pow(max(dot(V, -u_sun), 0.0), 2.0));
    sh = mix(1.0, shadow(u_sun * 3.0), 0.8);
  }
  // The two ends of the old screen-space sum, sky only and sky with full sun, stay where they were.
  vec3 sky = u_amb * (lit_sides ? 1.0 : 0.75);
  vec3 A = lin(sky);
  vec3 S = lin(sky + u_sun_col * 0.92) - A;
  vec3 lit = c.rgb * (A * ao * kA + S * (kD * ndl * sh * shade + kF * (0.4 + 0.6 * sh)) + kE);
  float alpha = u_decal != 0 ? c.a : u_alpha;
  float strength = max(spec.r, max(spec.g, spec.b));
  if (strength > 0.0) {
    vec3 H = normalize(V + u_sun);
    lit += S * spec * 0.5 * pow(max(dot(N, H), 0.0), gloss) * sh * step(0.0, dot(N, u_sun));
    float f = u_f0 + (1.0 - u_f0) * pow(1.0 - max(dot(N, V), 0.0), 5.0);
    vec3 env = lin(environment(reflect(-V, N)));
    if (u_mode == 5) {
      // Glass shows what is behind it head on and the sky at a slant.
      lit = mix(lit, env, f);
      alpha = max(c.a, f);
    } else {
      // Weathered surfaces mirror dimly and take on their own colour.
      lit += env * mix(vec3(1.0), c.rgb * 2.0, 0.6) * min(f, 0.6) * min(strength, 1.0) * 0.4 * ao;
    }
  }
  if (v_tint.a >= 0.99) lit = mix(lit, lin(v_tint.rgb), 0.55);
  lit = mix(lit, lin(vec3(0.73,0.79,0.86)), 1.0 - exp(-dist * u_fog));
  o = vec4(tone(pow(lit, vec3(1.0 / 2.2)) * u_exposure), v_tint.a >= 0.99 ? 1.0 : alpha);
}"#;

const SKY_VS: &str = r#"#version 330 core
layout(location=0) in vec2 a_pos;
out vec2 v;
void main(){ v = a_pos * 2.0 - 1.0; gl_Position = vec4(v, 1.0, 1.0); }"#;

const SKY_FS: &str = r#"#version 330 core
in vec2 v; out vec4 o;
uniform vec3 u_f, u_r, u_u, u_sun; uniform vec2 u_t; uniform float u_exposure, u_time;
float hash(vec2 p){ return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }
float noise(vec2 p){
  vec2 i = floor(p), f = fract(p);
  f = f * f * (3.0 - 2.0 * f);
  return mix(mix(hash(i), hash(i + vec2(1.0, 0.0)), f.x), mix(hash(i + vec2(0.0, 1.0)), hash(i + vec2(1.0, 1.0)), f.x), f.y);
}
float fbm(vec2 p){
  float v = 0.0, a = 0.5;
  for (int i = 0; i < 5; i++) { v += a * noise(p); p = p * 2.03 + vec2(17.0, 9.0); a *= 0.5; }
  return v;
}
void main(){
  vec3 d = normalize(u_f + u_r * v.x * u_t.x + u_u * v.y * u_t.y);
  float h = clamp(d.y, -0.05, 1.0);
  vec3 c = mix(vec3(0.73,0.79,0.86), vec3(0.38,0.53,0.74), pow(max(h, 0.0), 0.55));
  float day = smoothstep(-0.12, 0.3, u_sun.y);
  c = mix(vec3(0.09,0.11,0.2), c, day);
  // Warm band near the horizon when the sun is low.
  c = mix(c, vec3(0.95,0.6,0.38), (1.0 - smoothstep(0.0, 0.45, u_sun.y)) * smoothstep(0.35, 0.0, abs(h)) * 0.55 * smoothstep(-0.1, 0.05, u_sun.y));
  // High thin cloud, drifting slowly and thinning out towards the horizon haze.
  if (d.y > 0.0) {
    vec2 q = d.xz / (d.y + 0.12) * 0.9 + vec2(u_time * 0.004, u_time * 0.0015);
    float cover = smoothstep(0.42, 0.78, fbm(q * vec2(1.0, 2.2)) * 0.75 + fbm(q * 3.1) * 0.25);
    vec3 cloud = mix(vec3(0.62, 0.66, 0.72), vec3(0.95, 0.96, 0.97), day) * (0.8 + 0.2 * max(u_sun.y, 0.0));
    c = mix(c, cloud, cover * 0.7 * smoothstep(0.0, 0.22, d.y));
  }
  float s = max(dot(d, u_sun), 0.0);
  c += vec3(1.0,0.9,0.75) * (pow(s, 400.0) * 1.2 + pow(s, 12.0) * 0.08);
  o = vec4(c * u_exposure, 1.0);
}"#;

/// Ambient occlusion from the depth buffer and edge smoothing, drawn over the finished view.
const POST_FS: &str = r#"#version 330 core
in vec2 v; out vec4 o;
uniform sampler2D u_color, u_depth;
// Clip space to positions relative to the eye, and back.
uniform mat4 u_inv, u_proj;
uniform vec2 u_texel;
uniform int u_ao, u_fxaa;
uniform float u_ao_strength;
vec3 posAt(vec2 uv){
  vec4 p = u_inv * vec4(uv * 2.0 - 1.0, texture(u_depth, uv).r * 2.0 - 1.0, 1.0);
  return p.xyz / p.w;
}
float luma(vec3 c){ return dot(c, vec3(0.299, 0.587, 0.114)); }
vec3 fxaa(vec2 uv){
  vec3 m = texture(u_color, uv).rgb;
  float lnw = luma(texture(u_color, uv + vec2(-1.0, -1.0) * u_texel).rgb), lne = luma(texture(u_color, uv + vec2(1.0, -1.0) * u_texel).rgb);
  float lsw = luma(texture(u_color, uv + vec2(-1.0, 1.0) * u_texel).rgb), lse = luma(texture(u_color, uv + vec2(1.0, 1.0) * u_texel).rgb);
  float lm = luma(m);
  float lo = min(lm, min(min(lnw, lne), min(lsw, lse))), hi = max(lm, max(max(lnw, lne), max(lsw, lse)));
  if (hi - lo < max(0.03, hi * 0.125)) return m;
  vec2 dir = vec2(-((lnw + lne) - (lsw + lse)), (lnw + lsw) - (lne + lse));
  float reduce = max((lnw + lne + lsw + lse) * 0.03125, 0.0078125);
  dir = clamp(dir / (min(abs(dir.x), abs(dir.y)) + reduce), -8.0, 8.0) * u_texel;
  vec3 a = 0.5 * (texture(u_color, uv + dir * (1.0 / 3.0 - 0.5)).rgb + texture(u_color, uv + dir * (2.0 / 3.0 - 0.5)).rgb);
  vec3 b = a * 0.5 + 0.25 * (texture(u_color, uv - dir * 0.5).rgb + texture(u_color, uv + dir * 0.5).rgb);
  float lb = luma(b);
  return (lb < lo || lb > hi) ? a : b;
}
float hash(vec2 p){ return fract(52.9829189 * fract(dot(p, vec2(0.06711056, 0.00583715)))); }
void main(){
  vec2 uv = v * 0.5 + 0.5;
  vec3 col = u_fxaa != 0 ? fxaa(uv) : texture(u_color, uv).rgb;
  if (u_ao > 0 && texture(u_depth, uv).r < 1.0) {
    vec3 p = posAt(uv);
    float dist = length(p);
    float fade = 1.0 - smoothstep(110.0, 200.0, dist);
    if (fade > 0.0) {
      // The surface normal from the nearer neighbour on each axis, to stay off edges.
      vec3 px = posAt(uv + vec2(u_texel.x, 0.0)) - p, nx = p - posAt(uv - vec2(u_texel.x, 0.0));
      vec3 py = posAt(uv + vec2(0.0, u_texel.y)) - p, ny = p - posAt(uv - vec2(0.0, u_texel.y));
      vec3 n = normalize(cross(dot(px, px) < dot(nx, nx) ? px : nx, dot(py, py) < dot(ny, ny) ? py : ny));
      if (dot(n, p) > 0.0) n = -n;
      vec3 t = normalize(cross(n, abs(n.y) < 0.9 ? vec3(0.0, 1.0, 0.0) : vec3(1.0, 0.0, 0.0)));
      vec3 b = cross(n, t);
      float radius = clamp(dist * 0.06, 0.25, 1.5);
      float turn = hash(gl_FragCoord.xy) * 6.2832;
      float occ = 0.0;
      for (int i = 0; i < u_ao; i++) {
        float k = (float(i) + 0.5) / float(u_ao);
        float a = turn + float(i) * 2.39996;
        vec3 dir = (t * cos(a) + b * sin(a)) * sqrt(k) + n * sqrt(1.0 - k * 0.85);
        vec3 s = p + dir * radius * mix(0.25, 1.0, k);
        vec4 q = u_proj * vec4(s, 1.0);
        vec2 suv = q.xy / q.w * 0.5 + 0.5;
        if (q.w <= 0.0 || suv.x < 0.0 || suv.y < 0.0 || suv.x > 1.0 || suv.y > 1.0) continue;
        float seen = length(posAt(suv)), mine = length(s);
        // Only what lies close in front of the sample shades it.
        occ += step(seen, mine - 0.02 - dist * 0.002) * smoothstep(0.0, 1.0, radius / max(mine - seen, 1e-3));
      }
      col *= 1.0 - u_ao_strength * fade * occ / float(u_ao);
    }
  }
  o = vec4(col, 1.0);
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
uniform sampler2DArray u_tex; uniform vec3 u_eye, u_sun_col, u_amb; uniform float u_fog, u_exposure;
void main(){
  vec4 c = texture(u_tex, v_uv);
  if (c.a < 0.5) discard;
  vec3 lit = c.rgb * (u_amb * 0.8 + u_sun_col * 0.62);
  float dist = distance(v_world, u_eye);
  o = vec4(mix(lit, vec3(0.73,0.79,0.86), 1.0 - exp(-dist * u_fog)) * u_exposure, 1.0);
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
    pub shape: i32,
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
    /// (first index, index count, colour texture, shading)
    pub batches: Vec<(u32, u32, Option<usize>, Option<usize>)>,
    /// (name, width, height, RGBA)
    pub textures: Vec<(String, u32, u32, Vec<u8>)>,
    /// The compressed mipmaps of each texture, where the file has them; taken over the RGBA.
    pub raw: Vec<Option<Arc<forest_core::paa::Raw>>>,
    /// Number of visual LODs the model has.
    pub lod_count: u8,
    pub shadings: Vec<forest_core::model3d::Shading>,
    pub impostor: Option<Vec<(usize, Vec<u8>)>>,
    /// Side length and image centre of the impostor in model metres.
    pub impostor_fit: [f32; 3],
    /// Vertices follow the terrain, as road parts do in the game.
    pub conform: bool,
    pub water: bool,
    /// Lies flat on the ground and is blended over it.
    pub decal: bool,
}

pub const IMPOSTOR_SIZE: i32 = 192;
pub const IMPOSTOR_LAYERS: i32 = 1024;

/// A material with every slot resolved to a texture.
#[derive(Clone, Copy)]
struct MatGpu {
    /// `u_mode` of the mesh shader: 1 single layer, 2 Multi, 3 water, 4 foliage, 5 glass, 6 bark.
    mode: i32,
    maps: [glow::Texture; 15],
    xf: [f32; 60],
    xo: [f32; 45],
    reflectance: f32,
    /// Emissive, ambient, diffuse, forced diffuse, specular.
    colors: [f32; 15],
    /// Inverted channels of each slot's texture.
    inv: [f32; 60],
    power: f32,
    alpha_test: f32,
    blend: forest_core::model3d::Blend,
    top: [f32; 3],
}

/// When a batch is drawn within a frame.
const PASS_OPAQUE: u8 = 1;
const PASS_DECAL: u8 = 2;
const PASS_BLEND: u8 = 4;
const PASS_WATER: u8 = 8;

struct Model {
    vao: glow::VertexArray,
    vbo: glow::Buffer,
    ebo: glow::Buffer,
    batches: Vec<(u32, u32, Option<glow::Texture>, Option<MatGpu>)>,
    /// Pass of each batch, and all of them together.
    batch_pass: Vec<u8>,
    /// Inverted channels of each batch's colour texture.
    batch_inv: Vec<[f32; 4]>,
    passes: u8,
    triangles: u32,
    conform: bool,
    water: bool,
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
    /// Instances casting shadows, near the camera only.
    pub shadow_instances: Arc<Vec<f32>>,
    pub shadow_groups: Arc<Vec<((u32, u8), u32, u32)>>,
}

/// Lighting values the shaders use.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightGpu {
    pub sun: [f32; 3],
    pub sun_color: [f32; 3],
    pub ambient: [f32; 3],
    pub fog: f32,
    pub exposure: f32,
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
    pub surface_blend: f32,
    pub surface_range: f32,
    pub sea_waves: f32,
    pub world: f32,
    pub tile: f32,
    pub lighting: LightGpu,
    /// Seconds, drives the water ripples.
    pub time: f32,
    /// Sun view for the shadow map.
    pub light: Option<[f32; 16]>,
    /// Depth offset against shadow acne, in shadow map depth units.
    pub shadow_bias: f32,
    /// Samples of ambient occlusion per pixel; 0 turns it off.
    pub ao: u32,
    /// Edge smoothing: 0 off, 1 FXAA, 2 with the view drawn at 1.5 times its size, 3 at twice.
    pub smoothing: u32,
}

enum Upload {
    Holes { width: i32, height: i32, data: Vec<u8> },
    Reset { cols: i32, rows: i32, cell: f32, data: Vec<f32> },
    Height { x: i32, y: i32, w: i32, h: i32, data: Vec<f32> },
    Mask { width: i32, height: i32, data: Vec<u8> },
    MaskRect { x: i32, y: i32, w: i32, h: i32, data: Vec<u8> },
    Satellite { width: i32, height: i32, rgb: Vec<u8> },
    NoSatellite,
    Detail { width: i32, height: i32, rgb: Vec<u8>, rect: [f32; 4] },
    Palette { colors: Vec<[u8; 3]>, averages: Vec<[u8; 3]> },
    Layer { index: i32, size: i32, rgba: Vec<u8>, normal: Vec<u8> },
    Mesh(Box<MeshData>),
    ClearModels,
}

struct Programs {
    terrain: glow::Program,
    water: glow::Program,
    mesh: glow::Program,
    impostor: glow::Program,
    sky: glow::Program,
    post: glow::Program,
}

/// Off-screen image of a view, with its depth, for the passes drawn over it.
#[derive(Clone, Copy)]
struct Target {
    fbo: glow::Framebuffer,
    color: glow::Texture,
    depth: glow::Texture,
    size: [i32; 2],
}

/// Inverse of a 4 x 4 matrix, worked out in double precision.
fn invert(m: &[f64; 16]) -> Option<[f64; 16]> {
    let mut inv = [0.0f64; 16];
    inv[0] = m[5] * m[10] * m[15] - m[5] * m[11] * m[14] - m[9] * m[6] * m[15] + m[9] * m[7] * m[14] + m[13] * m[6] * m[11] - m[13] * m[7] * m[10];
    inv[4] = -m[4] * m[10] * m[15] + m[4] * m[11] * m[14] + m[8] * m[6] * m[15] - m[8] * m[7] * m[14] - m[12] * m[6] * m[11] + m[12] * m[7] * m[10];
    inv[8] = m[4] * m[9] * m[15] - m[4] * m[11] * m[13] - m[8] * m[5] * m[15] + m[8] * m[7] * m[13] + m[12] * m[5] * m[11] - m[12] * m[7] * m[9];
    inv[12] = -m[4] * m[9] * m[14] + m[4] * m[10] * m[13] + m[8] * m[5] * m[14] - m[8] * m[6] * m[13] - m[12] * m[5] * m[10] + m[12] * m[6] * m[9];
    inv[1] = -m[1] * m[10] * m[15] + m[1] * m[11] * m[14] + m[9] * m[2] * m[15] - m[9] * m[3] * m[14] - m[13] * m[2] * m[11] + m[13] * m[3] * m[10];
    inv[5] = m[0] * m[10] * m[15] - m[0] * m[11] * m[14] - m[8] * m[2] * m[15] + m[8] * m[3] * m[14] + m[12] * m[2] * m[11] - m[12] * m[3] * m[10];
    inv[9] = -m[0] * m[9] * m[15] + m[0] * m[11] * m[13] + m[8] * m[1] * m[15] - m[8] * m[3] * m[13] - m[12] * m[1] * m[11] + m[12] * m[3] * m[9];
    inv[13] = m[0] * m[9] * m[14] - m[0] * m[10] * m[13] - m[8] * m[1] * m[14] + m[8] * m[2] * m[13] + m[12] * m[1] * m[10] - m[12] * m[2] * m[9];
    inv[2] = m[1] * m[6] * m[15] - m[1] * m[7] * m[14] - m[5] * m[2] * m[15] + m[5] * m[3] * m[14] + m[13] * m[2] * m[7] - m[13] * m[3] * m[6];
    inv[6] = -m[0] * m[6] * m[15] + m[0] * m[7] * m[14] + m[4] * m[2] * m[15] - m[4] * m[3] * m[14] - m[12] * m[2] * m[7] + m[12] * m[3] * m[6];
    inv[10] = m[0] * m[5] * m[15] - m[0] * m[7] * m[13] - m[4] * m[1] * m[15] + m[4] * m[3] * m[13] + m[12] * m[1] * m[7] - m[12] * m[3] * m[5];
    inv[14] = -m[0] * m[5] * m[14] + m[0] * m[6] * m[13] + m[4] * m[1] * m[14] - m[4] * m[2] * m[13] - m[12] * m[1] * m[6] + m[12] * m[2] * m[5];
    inv[3] = -m[1] * m[6] * m[11] + m[1] * m[7] * m[10] + m[5] * m[2] * m[11] - m[5] * m[3] * m[10] - m[9] * m[2] * m[7] + m[9] * m[3] * m[6];
    inv[7] = m[0] * m[6] * m[11] - m[0] * m[7] * m[10] - m[4] * m[2] * m[11] + m[4] * m[3] * m[10] + m[8] * m[2] * m[7] - m[8] * m[3] * m[6];
    inv[11] = -m[0] * m[5] * m[11] + m[0] * m[7] * m[9] + m[4] * m[1] * m[11] - m[4] * m[3] * m[9] - m[8] * m[1] * m[7] + m[8] * m[3] * m[5];
    inv[15] = m[0] * m[5] * m[10] - m[0] * m[6] * m[9] - m[4] * m[1] * m[10] + m[4] * m[2] * m[9] + m[8] * m[1] * m[6] - m[8] * m[2] * m[5];
    let det = m[0] * inv[0] + m[1] * inv[4] + m[2] * inv[8] + m[3] * inv[12];
    (det.abs() > 1e-300).then(|| inv.map(|v| v / det))
}

/// The view-projection matrix for positions measured from the eye, and its inverse.
fn eye_relative(vp: &[f32; 16], eye: [f32; 3]) -> Option<([f32; 16], [f32; 16])> {
    let mut m: [f64; 16] = std::array::from_fn(|i| vp[i] as f64);
    for row in 0..4 {
        m[12 + row] += m[row] * eye[0] as f64 + m[4 + row] * eye[1] as f64 + m[8 + row] * eye[2] as f64;
    }
    let inv = invert(&m)?;
    Some((m.map(|v| v as f32), inv.map(|v| v as f32)))
}

#[derive(Default)]
pub struct Renderer {
    uploads: Vec<Upload>,
    programs: Option<Programs>,
    patch_vao: Option<glow::VertexArray>,
    patch_instances: Option<glow::Buffer>,
    index_count: i32,
    quad_vao: Option<glow::VertexArray>,
    sea_vao: Option<glow::VertexArray>,
    sea_indices: i32,
    quad_vbo: Option<glow::Buffer>,
    box_vao: Option<glow::VertexArray>,
    box_vbo: Option<glow::Buffer>,
    box_ebo: Option<glow::Buffer>,
    impostor_instances: Option<glow::Buffer>,
    mesh_instances: HashMap<u8, glow::Buffer>,
    box_instances: Option<glow::Buffer>,
    height: Option<glow::Texture>,
    mask: Option<glow::Texture>,
    holes: Option<glow::Texture>,
    sat: Option<glow::Texture>,
    detail: Option<glow::Texture>,
    detail_rect: [f32; 4],
    has_sat: bool,
    palette: Option<glow::Texture>,
    layers: Option<glow::Texture>,
    layer_normals: Option<glow::Texture>,
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
    shadow_fbo: Option<glow::Framebuffer>,
    shadow_map: Option<glow::Texture>,
    shadow_buffer: Option<glow::Buffer>,
    shadow_drawn: Option<*const Vec<f32>>,
    shadow_valid: bool,
    shadow_light: Option<[f32; 16]>,
    /// GPU textures by name, shared between models, with their inverted channels.
    shared: HashMap<String, (glow::Texture, [f32; 4])>,
    /// Visual LODs of each model, known once one of them has loaded.
    lod_counts: HashMap<u32, u8>,
    /// Anisotropic filtering of object and surface textures; below 2 is off.
    pub anisotropy: f32,
    /// Side of the shadow map in pixels; 0 takes the default.
    pub shadow_size: i32,
    shadow_alloc: i32,
    /// Off-screen image of each view.
    targets: HashMap<u8, Target>,
    fallback: Option<[glow::Texture; 5]>,
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

/// Texture unit of material slot `k` (units 0, 1 and 12 hold colour, heights and shadows).
fn material_unit(k: u32) -> u32 {
    if k <= 10 { 1 + k } else { 2 + k }
}

unsafe fn compile(gl: &glow::Context, vs: &str, fs: &str) -> Result<glow::Program, String> {
    let program = gl.create_program()?;
    let mut shaders = Vec::new();
    for (kind, src) in [(glow::VERTEX_SHADER, vs), (glow::FRAGMENT_SHADER, fs)] {
        let s = gl.create_shader(kind)?;
        gl.shader_source(s, &src.replace("//WATER", WATER_GLSL));
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
    pub fn set_holes(&mut self, holes: &dz_core::holes::Holes, cols: usize, rows: usize) {
        let mut data = Vec::new();
        if !holes.is_empty() {
            data = vec![0u8; cols * rows];
            for [x, y] in holes {
                if let Some(cell) = data.get_mut(*y as usize * cols + *x as usize) { *cell = 1; }
            }
        }
        self.uploads.push(Upload::Holes { width: cols as i32, height: rows as i32, data });
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
    pub fn set_layer(&mut self, index: usize, size: usize, rgba: Vec<u8>, normal: Vec<u8>) {
        self.uploads.push(Upload::Layer { index: index as i32, size: size as i32, rgba, normal });
    }
    pub fn add_mesh(&mut self, m: MeshData) {
        self.uploads.push(Upload::Mesh(Box::new(m)));
    }
    /// Number of visual LODs of a model, once any of them has loaded.
    pub fn lod_count(&self, model: u32) -> Option<u8> {
        self.lod_counts.get(&model).copied()
    }
    /// The loaded LOD nearest to the wanted one, coarser ones first, with its triangles.
    pub fn best_lod(&self, model: u32, want: u8) -> Option<(u8, u32)> {
        let last = self.lod_counts.get(&model).map_or(want, |c| c.saturating_sub(1)).max(want);
        (want..=last).chain((0..want).rev()).find_map(|l| self.models.get(&(model, l)).map(|m| (l, m.triangles)))
    }
    /// Drops every model and texture, so they load again with the current settings.
    pub fn clear_models(&mut self) {
        self.uploads.retain(|u| !matches!(u, Upload::Mesh(_)));
        self.uploads.push(Upload::ClearModels);
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
            post: compile(gl, SKY_VS, POST_FS)?,
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

        // Grid for the sea surface.
        const SEA: u32 = 360;
        let mut sea: Vec<f32> = Vec::with_capacity(((SEA + 1) * (SEA + 1) * 2) as usize);
        let mut sea_i: Vec<u32> = Vec::with_capacity((SEA * SEA * 6) as usize);
        for y in 0..=SEA {
            for x in 0..=SEA {
                sea.extend_from_slice(&[x as f32 / SEA as f32, y as f32 / SEA as f32]);
                if x < SEA && y < SEA {
                    let at = y * (SEA + 1) + x;
                    sea_i.extend_from_slice(&[at, at + 1, at + SEA + 1, at + 1, at + SEA + 2, at + SEA + 1]);
                }
            }
        }
        let svao = gl.create_vertex_array()?;
        gl.bind_vertex_array(Some(svao));
        let svbo = gl.create_buffer()?;
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(svbo));
        gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes(&sea), glow::STATIC_DRAW);
        let sebo = gl.create_buffer()?;
        gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(sebo));
        gl.buffer_data_u8_slice(glow::ELEMENT_ARRAY_BUFFER, bytes(&sea_i), glow::STATIC_DRAW);
        gl.enable_vertex_attrib_array(0);
        gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, 8, 0);
        self.sea_vao = Some(svao);
        self.sea_indices = sea_i.len() as i32;
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
            let base = (b.len() / 10) as u32;
            for (s, t) in [(-0.5f32, -0.5f32), (0.5, -0.5), (-0.5, 0.5), (0.5, 0.5)] {
                let p = [n[0] * 0.5 + u[0] * s + v[0] * t, n[1] * 0.5 + u[1] * s + v[1] * t + 0.5, n[2] * 0.5 + u[2] * s + v[2] * t];
                b.extend_from_slice(&[p[0], p[1], p[2], n[0], n[1], n[2], 0.0, 0.0, 0.0, 0.0]);
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
        let normals = gl.create_texture()?;
        gl.bind_texture(glow::TEXTURE_2D_ARRAY, Some(normals));
        gl.tex_image_3d(glow::TEXTURE_2D_ARRAY, 0, glow::RGBA8 as i32, 512, 512, 64, 0, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelUnpackData::Slice(None));
        gl.generate_mipmap(glow::TEXTURE_2D_ARRAY);
        set_filter(gl, glow::TEXTURE_2D_ARRAY, glow::LINEAR_MIPMAP_LINEAR, glow::LINEAR, glow::REPEAT);
        self.layer_normals = Some(normals);
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
        gl.vertex_attrib_pointer_f32(0, 3, glow::FLOAT, false, 40, 0);
        gl.enable_vertex_attrib_array(1);
        gl.vertex_attrib_pointer_f32(1, 3, glow::FLOAT, false, 40, 12);
        gl.enable_vertex_attrib_array(2);
        gl.vertex_attrib_pointer_f32(2, 2, glow::FLOAT, false, 40, 24);
        gl.enable_vertex_attrib_array(7);
        gl.vertex_attrib_pointer_f32(7, 2, glow::FLOAT, false, 40, 32);
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
                Upload::Holes { width, height, data } => {
                    if let Some(t) = self.holes.take() {
                        gl.delete_texture(t);
                    }
                    if !data.is_empty() {
                        let t = gl.create_texture()?;
                        gl.bind_texture(glow::TEXTURE_2D, Some(t));
                        gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
                        gl.tex_image_2d(glow::TEXTURE_2D, 0, glow::R8UI as i32, width, height, 0, glow::RED_INTEGER, glow::UNSIGNED_BYTE, glow::PixelUnpackData::Slice(Some(&data)));
                        set_filter(gl, glow::TEXTURE_2D, glow::NEAREST, glow::NEAREST, glow::CLAMP_TO_EDGE);
                        self.holes = Some(t);
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
                Upload::Layer { index, size, rgba, normal } => {
                    if index < 64 && size == 512 {
                        gl.bind_texture(glow::TEXTURE_2D_ARRAY, self.layers);
                        gl.tex_sub_image_3d(glow::TEXTURE_2D_ARRAY, 0, 0, 0, index, 512, 512, 1, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelUnpackData::Slice(Some(&rgba)));
                        gl.generate_mipmap(glow::TEXTURE_2D_ARRAY);
                        if normal.len() == rgba.len() {
                            gl.bind_texture(glow::TEXTURE_2D_ARRAY, self.layer_normals);
                            gl.tex_sub_image_3d(glow::TEXTURE_2D_ARRAY, 0, 0, 0, index, 512, 512, 1, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelUnpackData::Slice(Some(&normal)));
                            gl.generate_mipmap(glow::TEXTURE_2D_ARRAY);
                        }
                    }
                }
                Upload::ClearModels => {
                    for (_, m) in self.models.drain() {
                        gl.delete_vertex_array(m.vao);
                        gl.delete_buffer(m.vbo);
                        gl.delete_buffer(m.ebo);
                    }
                    for (_, (t, _)) in self.shared.drain() {
                        gl.delete_texture(t);
                    }
                    self.lod_counts.clear();
                    self.impostor_layers.clear();
                    self.next_impostor = 0;
                }
                Upload::Mesh(m) => {
                    // A few per frame keep the interface smooth while models stream in.
                    if meshes_this_frame >= 2 {
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
        self.lod_counts.insert(m.key.0, m.lod_count.max(1));
        // Two requests can end at the same LOD of a model with few of them.
        if m.indices.is_empty() || self.models.contains_key(&m.key) {
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
        let mut textures: Vec<(glow::Texture, [f32; 4])> = Vec::with_capacity(m.textures.len());
        for (i, (name, w, h, rgba)) in m.textures.iter().enumerate() {
            let t = match self.shared.get(name) {
                Some(t) => *t,
                None => {
                    let t = gl.create_texture()?;
                    gl.bind_texture(glow::TEXTURE_2D, Some(t));
                    let mut inv = [0.0f32; 4];
                    match m.raw.get(i).and_then(|r| r.as_ref()) {
                        Some(raw) => {
                            // The file's own DXT mipmaps go to the card as they are.
                            let format = if raw.dxt5 { glow::COMPRESSED_RGBA_S3TC_DXT5_EXT } else { glow::COMPRESSED_RGBA_S3TC_DXT1_EXT };
                            for (level, (w, h, data)) in raw.mips.iter().enumerate() {
                                gl.compressed_tex_image_2d(glow::TEXTURE_2D, level as i32, format as i32, *w as i32, *h as i32, 0, data.len() as i32, data);
                            }
                            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAX_LEVEL, raw.mips.len() as i32 - 1);
                            // Channels are rearranged when sampled; an inverted one is left to the shader.
                            for (target, param, channel) in [(1, glow::TEXTURE_SWIZZLE_R, 0), (2, glow::TEXTURE_SWIZZLE_G, 1), (3, glow::TEXTURE_SWIZZLE_B, 2), (0, glow::TEXTURE_SWIZZLE_A, 3)] {
                                let source = raw.swizzle[target];
                                let from = match source {
                                    0 | 4 => glow::ALPHA,
                                    1 | 5 => glow::RED,
                                    2 | 6 => glow::GREEN,
                                    3 | 7 => glow::BLUE,
                                    _ => glow::ONE,
                                };
                                gl.tex_parameter_i32(glow::TEXTURE_2D, param, from as i32);
                                if (4..8).contains(&source) {
                                    inv[channel] = 1.0;
                                }
                            }
                            set_filter(gl, glow::TEXTURE_2D, if raw.mips.len() > 1 { glow::LINEAR_MIPMAP_LINEAR } else { glow::LINEAR }, glow::LINEAR, glow::REPEAT);
                        }
                        None => {
                            gl.tex_image_2d(glow::TEXTURE_2D, 0, glow::RGBA8 as i32, *w as i32, *h as i32, 0, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelUnpackData::Slice(Some(rgba)));
                            gl.generate_mipmap(glow::TEXTURE_2D);
                            set_filter(gl, glow::TEXTURE_2D, glow::LINEAR_MIPMAP_LINEAR, glow::LINEAR, glow::REPEAT);
                        }
                    }
                    if self.anisotropy >= 2.0 {
                        gl.tex_parameter_f32(glow::TEXTURE_2D, glow::TEXTURE_MAX_ANISOTROPY, self.anisotropy.min(16.0));
                    }
                    self.shared.insert(name.clone(), (t, inv));
                    (t, inv)
                }
            };
            textures.push(t);
        }
        let fallback = match self.fallback {
            Some(f) => f,
            None => {
                let mut list = Vec::new();
                for rgba in [[255u8, 255, 255, 255], [128, 128, 128, 255], [128, 128, 255, 255], [0, 0, 0, 0], [128, 0, 255, 255]] {
                    let t = gl.create_texture()?;
                    gl.bind_texture(glow::TEXTURE_2D, Some(t));
                    gl.tex_image_2d(glow::TEXTURE_2D, 0, glow::RGBA8 as i32, 1, 1, 0, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelUnpackData::Slice(Some(&rgba)));
                    set_filter(gl, glow::TEXTURE_2D, glow::NEAREST, glow::NEAREST, glow::REPEAT);
                    list.push(t);
                }
                let f = [list[0], list[1], list[2], list[3], list[4]];
                self.fallback = Some(f);
                f
            }
        };
        let [white, gray, flat, clear, dtsmdi] = fallback;
        use forest_core::model3d::{Blend, ShadingKind};
        let mut batches = Vec::with_capacity(m.batches.len());
        let mut batch_pass = Vec::with_capacity(m.batches.len());
        let mut batch_inv = Vec::with_capacity(m.batches.len());
        for (f, c, t, sh) in &m.batches {
            let base_slot = t.and_then(|i| textures.get(i).copied());
            let base = base_slot.map(|b| b.0);
            let shading = sh.and_then(|i| m.shadings.get(i));
            // Passes that only feed the game's depth or shadow buffers have nothing to show.
            if shading.is_some_and(|s| s.hidden) {
                continue;
            }
            let mat = shading.map(|s| {
                let defaults = match s.kind {
                    ShadingKind::Multi => [gray, gray, gray, gray, clear, clear, white, flat, flat, flat, flat, dtsmdi, dtsmdi, dtsmdi, dtsmdi],
                    _ => [gray, flat, gray, clear, white, white, gray, gray, gray, gray, gray, gray, gray, gray, gray],
                };
                let mut maps = defaults;
                if s.kind == ShadingKind::Tree {
                    maps[3] = gray;
                }
                let mut inv = [0.0f32; 60];
                for k in 0..15 {
                    if let Some((tex, flip)) = s.maps[k].and_then(|i| textures.get(i).copied()) {
                        maps[k] = tex;
                        inv[k * 4..k * 4 + 4].copy_from_slice(&flip);
                    }
                }
                if !s.multi() {
                    maps[0] = base.unwrap_or(gray);
                    inv[..4].copy_from_slice(&base_slot.map_or([0.0; 4], |b| b.1));
                }
                let (mut xf, mut xo) = ([0.0f32; 60], [0.0f32; 45]);
                for k in 0..15 {
                    xf[k * 4..k * 4 + 4].copy_from_slice(&s.xf[k]);
                    xo[k * 3..k * 3 + 3].copy_from_slice(&s.xo[k]);
                }
                let mut colors = [0.0f32; 15];
                for (k, v) in [s.emissive, s.ambient, s.diffuse, s.forced_diffuse, s.specular].iter().enumerate() {
                    colors[k * 3..k * 3 + 3].copy_from_slice(v);
                }
                let mode = match s.kind {
                    ShadingKind::Super => 1,
                    ShadingKind::Multi => 2,
                    ShadingKind::Water => 3,
                    ShadingKind::Tree if s.trunk => 6,
                    ShadingKind::Tree => 4,
                    ShadingKind::Glass => 5,
                };
                MatGpu { mode, maps, xf, xo, reflectance: s.reflectance, colors, inv, power: s.specular_power, alpha_test: s.alpha_test, blend: s.blend, top: s.top.unwrap_or([-1.0, 0.0, 0.0]) }
            });
            batch_pass.push(if m.water || mat.as_ref().is_some_and(|x| x.mode == 3) {
                PASS_WATER
            } else if m.decal {
                PASS_DECAL
            } else if mat.as_ref().is_some_and(|x| x.blend != Blend::Opaque) {
                PASS_BLEND
            } else {
                PASS_OPAQUE
            });
            batch_inv.push(base_slot.map_or([0.0; 4], |b| b.1));
            batches.push((*f, *c, base, mat));
        }
        let passes = batch_pass.iter().fold(0, |a, b| a | b);
        let triangles = m.indices.len() as u32 / 3;
        self.models.insert(m.key, Model { vao, vbo, ebo, batches, batch_pass, batch_inv, passes, triangles, conform: m.conform, water: m.water });
        Ok(())
    }

    /// The off-screen image of a view, made anew when the view changes size.
    unsafe fn target(&mut self, gl: &glow::Context, key: u8, w: i32, h: i32) -> Result<Target, String> {
        if let Some(t) = self.targets.get(&key).filter(|t| t.size == [w, h]) {
            return Ok(*t);
        }
        if let Some(old) = self.targets.remove(&key) {
            gl.delete_framebuffer(old.fbo);
            gl.delete_texture(old.color);
            gl.delete_texture(old.depth);
        }
        let color = gl.create_texture()?;
        gl.bind_texture(glow::TEXTURE_2D, Some(color));
        gl.tex_image_2d(glow::TEXTURE_2D, 0, glow::RGBA8 as i32, w, h, 0, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelUnpackData::Slice(None));
        set_filter(gl, glow::TEXTURE_2D, glow::LINEAR, glow::LINEAR, glow::CLAMP_TO_EDGE);
        let depth = gl.create_texture()?;
        gl.bind_texture(glow::TEXTURE_2D, Some(depth));
        gl.tex_image_2d(glow::TEXTURE_2D, 0, glow::DEPTH_COMPONENT24 as i32, w, h, 0, glow::DEPTH_COMPONENT, glow::UNSIGNED_INT, glow::PixelUnpackData::Slice(None));
        set_filter(gl, glow::TEXTURE_2D, glow::NEAREST, glow::NEAREST, glow::CLAMP_TO_EDGE);
        let fbo = gl.create_framebuffer()?;
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
        gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(color), 0);
        gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::DEPTH_ATTACHMENT, glow::TEXTURE_2D, Some(depth), 0);
        if gl.check_framebuffer_status(glow::FRAMEBUFFER) != glow::FRAMEBUFFER_COMPLETE {
            gl.delete_framebuffer(fbo);
            gl.delete_texture(color);
            gl.delete_texture(depth);
            return Err("framebuffer incomplete".into());
        }
        let t = Target { fbo, color, depth, size: [w, h] };
        self.targets.insert(key, t);
        Ok(t)
    }

    /// Renders what casts shadows from the sun into the depth map.
    unsafe fn draw_shadows(&mut self, gl: &glow::Context, f: &Frame, light: &[f32; 16], program: glow::Program, back: Option<glow::Framebuffer>) -> Result<(), String> {
        let size = if self.shadow_size >= 512 { self.shadow_size.min(self.max_tex.max(512)) } else { 4096 };
        if self.shadow_alloc != size {
            if let Some(t) = self.shadow_map.take() {
                gl.delete_texture(t);
            }
            if let Some(f) = self.shadow_fbo.take() {
                gl.delete_framebuffer(f);
            }
            self.shadow_alloc = size;
        }
        #[allow(non_snake_case)]
        let SIZE = size;
        if self.shadow_fbo.is_none() {
            let t = gl.create_texture()?;
            gl.bind_texture(glow::TEXTURE_2D, Some(t));
            gl.tex_image_2d(glow::TEXTURE_2D, 0, glow::DEPTH_COMPONENT24 as i32, SIZE, SIZE, 0, glow::DEPTH_COMPONENT, glow::UNSIGNED_INT, glow::PixelUnpackData::Slice(None));
            set_filter(gl, glow::TEXTURE_2D, glow::LINEAR, glow::LINEAR, glow::CLAMP_TO_EDGE);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_COMPARE_MODE, glow::COMPARE_REF_TO_TEXTURE as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_COMPARE_FUNC, glow::LEQUAL as i32);
            let fbo = gl.create_framebuffer()?;
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::DEPTH_ATTACHMENT, glow::TEXTURE_2D, Some(t), 0);
            gl.draw_buffer(glow::NONE);
            gl.read_buffer(glow::NONE);
            self.shadow_map = Some(t);
            self.shadow_fbo = Some(fbo);
            if self.shadow_buffer.is_none() {
                self.shadow_buffer = Some(gl.create_buffer()?);
            }
        }
        let mut viewport = [0i32; 4];
        gl.get_parameter_i32_slice(glow::VIEWPORT, &mut viewport);
        let scissor = gl.is_enabled(glow::SCISSOR_TEST);
        gl.disable(glow::SCISSOR_TEST);
        gl.bind_framebuffer(glow::FRAMEBUFFER, self.shadow_fbo);
        gl.viewport(0, 0, SIZE, SIZE);
        gl.enable(glow::DEPTH_TEST);
        gl.depth_func(glow::LEQUAL);
        gl.depth_mask(true);
        gl.disable(glow::BLEND);
        gl.disable(glow::CULL_FACE);
        gl.clear_depth_f32(1.0);
        gl.clear(glow::DEPTH_BUFFER_BIT);
        gl.use_program(Some(program));
        let u = |n: &str| gl.get_uniform_location(program, n);
        gl.uniform_matrix_4_f32_slice(u("u_vp").as_ref(), false, light);
        gl.uniform_1_i32(u("u_conform").as_ref(), 0);
        gl.uniform_1_i32(u("u_mode").as_ref(), 0);
        gl.uniform_1_i32(u("u_has_shadow").as_ref(), 0);
        gl.uniform_1_i32(u("u_tex").as_ref(), 0);
        gl.uniform_1_f32(u("u_alpha").as_ref(), 1.0);
        gl.uniform_4_f32_slice(u("u_inv").as_ref(), &[0.0; 60]);
        gl.uniform_3_f32_slice(u("u_eye").as_ref(), &f.view.eye);
        let textured = u("u_textured");
        gl.active_texture(glow::TEXTURE0);
        let buffer = self.shadow_buffer.unwrap();
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(buffer));
        gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes(&f.objects.shadow_instances), glow::STREAM_DRAW);
        for (key, first, count) in f.objects.shadow_groups.iter() {
            let Some(m) = self.models.get(key).filter(|m| !m.water && m.passes & (PASS_OPAQUE | PASS_DECAL) != 0) else { continue };
            gl.bind_vertex_array(Some(m.vao));
            Self::bind_mesh_instances(gl, buffer, *first);
            for ((start, n, tex, _), pass) in m.batches.iter().zip(&m.batch_pass) {
                // Glass and overlays let the sun through.
                if *pass != PASS_OPAQUE && *pass != PASS_DECAL {
                    continue;
                }
                gl.bind_texture(glow::TEXTURE_2D, *tex);
                gl.uniform_1_i32(textured.as_ref(), tex.is_some() as i32);
                gl.draw_elements_instanced(glow::TRIANGLES, *n as i32, glow::UNSIGNED_INT, (*start * 4) as i32, *count as i32);
            }
        }
        gl.bind_framebuffer(glow::FRAMEBUFFER, back);
        gl.viewport(viewport[0], viewport[1], viewport[2], viewport[3]);
        if scissor {
            gl.enable(glow::SCISSOR_TEST);
        }
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
        let (terrain, water, mesh_prog, impostor_prog, sky, post) = (p.terrain, p.water, p.mesh, p.impostor, p.sky, p.post);
        // Where the interface is being drawn; the view goes back there when it is finished.
        let screen = gl.get_parameter_framebuffer(glow::DRAW_FRAMEBUFFER_BINDING);
        let mut stats = Stats { models: self.models.len() as u32, ..Default::default() };
        let active_light = f.light.filter(|_| !f.objects.shadow_groups.is_empty());
        self.shadow_valid = false;
        if let Some(light) = &active_light {
            let ptr = Arc::as_ptr(&f.objects.shadow_instances);
            if self.shadow_drawn != Some(ptr) || self.shadow_light.as_ref() != Some(light) {
                self.shadow_light = Some(*light);
                if let Err(e) = self.draw_shadows(gl, f, light, mesh_prog, screen) {
                    self.error = Some(e);
                }
                self.shadow_drawn = Some(ptr);
            }
            self.shadow_valid = self.shadow_map.is_some();
        }

        // Ambient occlusion and edge smoothing work on a finished image, so the view is drawn
        // off screen first, larger than shown when it is to be scaled down.
        let mut area = [0i32; 4];
        gl.get_parameter_i32_slice(glow::VIEWPORT, &mut area);
        let clipped = gl.is_enabled(glow::SCISSOR_TEST);
        let scale = match f.smoothing {
            2 => 1.5,
            3 => 2.0,
            _ => 1.0,
        };
        let (tw, th) = (((area[2] as f32 * scale) as i32).clamp(1, self.max_tex.max(1)), ((area[3] as f32 * scale) as i32).clamp(1, self.max_tex.max(1)));
        let target = if f.ao > 0 || f.smoothing > 0 { self.target(gl, f.viewport_key, tw, th).ok() } else { None };
        if let Some(t) = &target {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(t.fbo));
            gl.viewport(0, 0, t.size[0], t.size[1]);
            gl.disable(glow::SCISSOR_TEST);
        }
        gl.enable(glow::DEPTH_TEST);
        gl.depth_func(glow::LEQUAL);
        gl.depth_mask(true);
        gl.clear_depth_f32(1.0);
        gl.clear_color(0.60, 0.70, 0.82, 1.0);
        gl.clear(glow::DEPTH_BUFFER_BIT | glow::COLOR_BUFFER_BIT);
        gl.disable(glow::CULL_FACE);
        gl.disable(glow::BLEND);

        let u = |prog: glow::Program, n: &str| gl.get_uniform_location(prog, n);
        let light_uniforms = |prog: glow::Program| {
            let l = &f.lighting;
            gl.uniform_3_f32_slice(u(prog, "u_sun").as_ref(), &l.sun);
            gl.uniform_3_f32_slice(u(prog, "u_sun_col").as_ref(), &l.sun_color);
            gl.uniform_3_f32_slice(u(prog, "u_amb").as_ref(), &l.ambient);
            gl.uniform_1_f32(u(prog, "u_fog").as_ref(), l.fog);
            gl.uniform_1_f32(u(prog, "u_exposure").as_ref(), l.exposure);
            gl.uniform_1_f32(u(prog, "u_time").as_ref(), f.time);
        };
        if !f.view.ortho {
            gl.depth_mask(false);
            gl.disable(glow::DEPTH_TEST);
            gl.use_program(Some(sky));
            light_uniforms(sky);
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
        light_uniforms(terrain);
        gl.uniform_matrix_4_f32_slice(u(terrain, "u_vp").as_ref(), false, &f.view.vp);
        gl.uniform_1_f32(u(terrain, "u_transition").as_ref(), f.surface_blend);
        gl.uniform_1_f32(u(terrain, "u_surface_range").as_ref(), f.surface_range);
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
                gl.uniform_1_i32(u(terrain, "u_brush_shape").as_ref(), b.shape);
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
        // Samplers of different types may not share a unit, so the shadow map always has its own.
        bind(6, glow::TEXTURE_2D, self.shadow_map, "u_shadow");
        bind(7, glow::TEXTURE_2D, self.holes.or(Some(mask)), "u_holes");
        bind(8, glow::TEXTURE_2D_ARRAY, self.layer_normals, "u_layer_normals");
        gl.uniform_1_i32(u(terrain, "u_has_holes").as_ref(), self.holes.is_some() as i32);
        gl.uniform_1_i32(u(terrain, "u_has_shadow").as_ref(), self.shadow_valid as i32);
        gl.uniform_1_f32(u(terrain, "u_bias").as_ref(), f.shadow_bias);
        if let (Some(l), true) = (&active_light, self.shadow_valid) {
            gl.uniform_matrix_4_f32_slice(u(terrain, "u_light").as_ref(), false, l);
        }
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
        light_uniforms(mesh_prog);
        gl.uniform_matrix_4_f32_slice(u(mesh_prog, "u_vp").as_ref(), false, &f.view.vp);
        gl.uniform_3_f32(u(mesh_prog, "u_eye").as_ref(), f.view.eye[0], f.view.eye[1], f.view.eye[2]);
        gl.uniform_1_i32(u(mesh_prog, "u_tex").as_ref(), 0);
        gl.uniform_1_f32(u(mesh_prog, "u_alpha").as_ref(), 1.0);
        for k in 1..=14u32 {
            gl.uniform_1_i32(u(mesh_prog, &format!("u_m{k}")).as_ref(), material_unit(k) as i32);
        }
        gl.uniform_1_f32(u(mesh_prog, "u_bias").as_ref(), f.shadow_bias);
        let f0_u = u(mesh_prog, "u_f0");
        gl.uniform_1_i32(u(mesh_prog, "u_shadow").as_ref(), 12);
        gl.active_texture(glow::TEXTURE12);
        gl.bind_texture(glow::TEXTURE_2D, self.shadow_map);
        gl.uniform_1_i32(u(mesh_prog, "u_has_shadow").as_ref(), self.shadow_valid as i32);
        if let (Some(l), true) = (&active_light, self.shadow_valid) {
            gl.uniform_matrix_4_f32_slice(u(mesh_prog, "u_light").as_ref(), false, l);
        }
        gl.active_texture(glow::TEXTURE0);
        let mode_u = u(mesh_prog, "u_mode");
        gl.uniform_1_i32(mode_u.as_ref(), 0);
        let (xf_u, xo_u) = (u(mesh_prog, "u_xf"), u(mesh_prog, "u_xo"));
        gl.uniform_1_i32(u(mesh_prog, "u_height").as_ref(), 1);
        gl.uniform_2_f32(u(mesh_prog, "u_size").as_ref(), self.size[0], self.size[1]);
        gl.uniform_1_f32(u(mesh_prog, "u_cell").as_ref(), self.cell);
        gl.active_texture(glow::TEXTURE1);
        gl.bind_texture(glow::TEXTURE_2D, Some(height));
        gl.active_texture(glow::TEXTURE0);
        let conform_u = u(mesh_prog, "u_conform");
        gl.uniform_1_i32(conform_u.as_ref(), 0);
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
            let (mc_u, power_u, cut_u, top_u, decal_u) = (u(mesh_prog, "u_mc"), u(mesh_prog, "u_power"), u(mesh_prog, "u_cut"), u(mesh_prog, "u_top"), u(mesh_prog, "u_decal"));
            let inv_u = u(mesh_prog, "u_inv");
            gl.uniform_1_f32(cut_u.as_ref(), 0.0);
            gl.uniform_3_f32(top_u.as_ref(), -1.0, 0.0, 0.0);
            // Solid surfaces first, then what blends over them: decals, glass and overlays, water.
            for pass in [PASS_OPAQUE, PASS_DECAL, PASS_BLEND, PASS_WATER] {
                if pass != PASS_OPAQUE {
                    gl.enable(glow::BLEND);
                    gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
                    gl.depth_mask(false);
                }
                if pass == PASS_DECAL || pass == PASS_BLEND {
                    gl.enable(glow::POLYGON_OFFSET_FILL);
                    gl.polygon_offset(-2.0, -2.0);
                } else {
                    gl.disable(glow::POLYGON_OFFSET_FILL);
                }
                gl.uniform_1_i32(decal_u.as_ref(), (pass == PASS_DECAL || pass == PASS_BLEND) as i32);
                for (key, first, count) in f.objects.groups.iter() {
                    let Some(m) = self.models.get(key) else { continue };
                    if pass == PASS_OPAQUE {
                        stats.triangles += m.triangles as u64 * *count as u64;
                        stats.mesh_instances += *count;
                    }
                    if m.passes & pass == 0 {
                        continue;
                    }
                    gl.bind_vertex_array(Some(m.vao));
                    Self::bind_mesh_instances(gl, buffer, *first);
                    gl.uniform_1_i32(conform_u.as_ref(), m.conform as i32);
                    for (((start, n, tex, mat), batch_pass), flip) in m.batches.iter().zip(&m.batch_pass).zip(&m.batch_inv) {
                        if *batch_pass != pass {
                            continue;
                        }
                        match mat {
                            _ if pass == PASS_WATER => gl.uniform_1_i32(mode_u.as_ref(), 3),
                            Some(mt) => {
                                gl.uniform_1_i32(mode_u.as_ref(), mt.mode);
                                gl.uniform_4_f32_slice(xf_u.as_ref(), &mt.xf);
                                gl.uniform_3_f32_slice(xo_u.as_ref(), &mt.xo);
                                gl.uniform_1_f32(f0_u.as_ref(), mt.reflectance);
                                gl.uniform_3_f32_slice(mc_u.as_ref(), &mt.colors);
                                gl.uniform_4_f32_slice(inv_u.as_ref(), &mt.inv);
                                gl.uniform_1_f32(power_u.as_ref(), mt.power);
                                gl.uniform_1_f32(cut_u.as_ref(), mt.alpha_test);
                                gl.uniform_3_f32_slice(top_u.as_ref(), &mt.top);
                                if pass == PASS_BLEND {
                                    let additive = mt.blend == forest_core::model3d::Blend::Add;
                                    gl.blend_func(glow::SRC_ALPHA, if additive { glow::ONE } else { glow::ONE_MINUS_SRC_ALPHA });
                                }
                                for k in 1..15u32 {
                                    gl.active_texture(glow::TEXTURE0 + material_unit(k));
                                    gl.bind_texture(glow::TEXTURE_2D, Some(mt.maps[k as usize]));
                                }
                                gl.active_texture(glow::TEXTURE0);
                                gl.bind_texture(glow::TEXTURE_2D, Some(mt.maps[0]));
                            }
                            None => {
                                gl.uniform_1_i32(mode_u.as_ref(), 0);
                                gl.uniform_1_f32(cut_u.as_ref(), 0.0);
                                gl.uniform_4_f32_slice(inv_u.as_ref(), flip);
                                gl.bind_texture(glow::TEXTURE_2D, *tex);
                            }
                        }
                        gl.uniform_1_i32(textured.as_ref(), tex.is_some() as i32);
                        gl.draw_elements_instanced(glow::TRIANGLES, *n as i32, glow::UNSIGNED_INT, (*start * 4) as i32, *count as i32);
                        stats.draw_calls += 1;
                    }
                }
            }
            gl.disable(glow::POLYGON_OFFSET_FILL);
            gl.depth_mask(true);
            gl.disable(glow::BLEND);
            gl.uniform_1_i32(decal_u.as_ref(), 0);
            gl.uniform_1_f32(cut_u.as_ref(), 0.0);
        }
        gl.uniform_1_i32(conform_u.as_ref(), 0);
        gl.uniform_1_i32(mode_u.as_ref(), 0);
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
            light_uniforms(impostor_prog);
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
            light_uniforms(water);
            gl.uniform_matrix_4_f32_slice(u(water, "u_vp").as_ref(), false, &f.view.vp);
            gl.uniform_3_f32(u(water, "u_eye").as_ref(), f.view.eye[0], f.view.eye[1], f.view.eye[2]);
            gl.uniform_1_f32(u(water, "u_sea").as_ref(), f.sea);
            gl.uniform_1_f32(u(water, "u_waves").as_ref(), f.sea_waves);
            gl.uniform_1_f32(u(water, "u_cell").as_ref(), self.cell);
            gl.uniform_2_f32(u(water, "u_size").as_ref(), self.size[0], self.size[1]);
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(height));
            gl.uniform_1_i32(u(water, "u_height").as_ref(), 0);
            gl.uniform_4_f32(u(water, "u_area").as_ref(), 0.0, 0.0, f.world * 4.0, f.world * 4.0);
            gl.bind_vertex_array(self.sea_vao);
            gl.draw_elements(glow::TRIANGLES, self.sea_indices, glow::UNSIGNED_INT, 0);
            gl.depth_mask(true);
        }
        if let Some(t) = target {
            gl.bind_framebuffer(glow::FRAMEBUFFER, screen);
            gl.viewport(area[0], area[1], area[2], area[3]);
            if clipped {
                gl.enable(glow::SCISSOR_TEST);
            }
            gl.disable(glow::DEPTH_TEST);
            gl.disable(glow::BLEND);
            gl.use_program(Some(post));
            gl.active_texture(glow::TEXTURE1);
            gl.bind_texture(glow::TEXTURE_2D, Some(t.depth));
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(t.color));
            gl.uniform_1_i32(u(post, "u_color").as_ref(), 0);
            gl.uniform_1_i32(u(post, "u_depth").as_ref(), 1);
            gl.uniform_2_f32(u(post, "u_texel").as_ref(), 1.0 / t.size[0] as f32, 1.0 / t.size[1] as f32);
            gl.uniform_1_i32(u(post, "u_fxaa").as_ref(), (f.smoothing > 0) as i32);
            let matrices = eye_relative(&f.view.vp, f.view.eye).filter(|_| !f.view.ortho);
            gl.uniform_1_i32(u(post, "u_ao").as_ref(), if matrices.is_some() { f.ao.min(32) as i32 } else { 0 });
            gl.uniform_1_f32(u(post, "u_ao_strength").as_ref(), 0.75);
            if let Some((proj, inv)) = matrices {
                gl.uniform_matrix_4_f32_slice(u(post, "u_proj").as_ref(), false, &proj);
                gl.uniform_matrix_4_f32_slice(u(post, "u_inv").as_ref(), false, &inv);
            }
            gl.bind_vertex_array(self.quad_vao);
            gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
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
