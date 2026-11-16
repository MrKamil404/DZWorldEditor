//! The material (`.rvmat`) of a surface layer, and the textures it is made of.
//!
//! The terrain takes two textures of each layer: the colour (`_ca`) and the relief (`_nopx`:
//! a normal map with the height for parallax in its alpha). The layer's material names both,
//! with how often they repeat and its lighting.
use anyhow::{bail, Context, Result};
use std::path::Path;

use crate::surfaces::{parse, Value};

#[derive(Clone, Debug, PartialEq)]
pub struct LayerMaterial {
    pub ambient: [f32; 4],
    pub diffuse: [f32; 4],
    pub forced_diffuse: [f32; 4],
    pub specular: [f32; 4],
    pub emmisive: [f32; 4],
    pub specular_power: f32,
    pub parallax_scale: f32,
    pub pixel_shader: String,
    pub vertex_shader: String,
    /// Relief texture (Stage1), relative to the work drive or a procedural one.
    pub nopx: String,
    /// Colour texture (Stage2).
    pub texture: String,
    /// How many times the textures repeat over their uv space.
    pub tiling: f32,
    /// Other top level lines, kept as they were.
    pub other: Vec<(String, String)>,
}

pub const FLAT_NOPX: &str = "#(argb,1,1,1)color(0.5,0.5,1,0.5,nopx)";

impl Default for LayerMaterial {
    fn default() -> Self {
        Self {
            ambient: [1.0; 4],
            diffuse: [0.7, 0.7, 0.7, 1.0],
            forced_diffuse: [0.0; 4],
            specular: [0.0; 4],
            emmisive: [0.0; 4],
            specular_power: 1.0,
            parallax_scale: 0.75,
            pixel_shader: "NormalMapDiffuse".into(),
            vertex_shader: "NormalMapDiffuseAlpha".into(),
            nopx: FLAT_NOPX.into(),
            texture: String::new(),
            tiling: 10.0,
            other: vec![("terrainIndirectLocalShadowBias".into(), "0".into())],
        }
    }
}

const KNOWN: [&str; 10] = ["ambient", "diffuse", "forcedDiffuse", "specular", "emmisive", "specularPower", "parallaxScale", "PixelShaderID", "VertexShaderID", "class"];

fn color(v: &Value, d: [f32; 4]) -> [f32; 4] {
    match v {
        Value::List(l) => std::array::from_fn(|i| l.get(i).and_then(|x| x.trim().parse().ok()).unwrap_or(d[i])),
        Value::Text(_) => d,
    }
}

fn num(x: f32) -> String {
    let s = format!("{x:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-" { "0".into() } else { s.to_string() }
}

impl LayerMaterial {
    /// A material for a colour texture, with the relief given or a flat one.
    pub fn for_texture(texture: &str, nopx: Option<&str>) -> Self {
        Self { texture: texture.to_string(), nopx: nopx.map_or_else(|| FLAT_NOPX.to_string(), str::to_string), ..Default::default() }
    }

    pub fn parse(text: &str) -> Self {
        let root = parse(text);
        let mut m = Self { other: Vec::new(), ..Default::default() };
        for (k, v) in &root.values {
            match k.as_str() {
                "ambient" => m.ambient = color(v, m.ambient),
                "diffuse" => m.diffuse = color(v, m.diffuse),
                "forcedDiffuse" => m.forced_diffuse = color(v, m.forced_diffuse),
                "specular" => m.specular = color(v, m.specular),
                "emmisive" => m.emmisive = color(v, m.emmisive),
                _ => match v {
                    Value::Text(t) => match k.as_str() {
                        "specularPower" => m.specular_power = t.parse().unwrap_or(m.specular_power),
                        "parallaxScale" => m.parallax_scale = t.parse().unwrap_or(m.parallax_scale),
                        "PixelShaderID" => m.pixel_shader = t.clone(),
                        "VertexShaderID" => m.vertex_shader = t.clone(),
                        _ if !KNOWN.contains(&k.as_str()) => m.other.push((k.clone(), t.clone())),
                        _ => {}
                    },
                    Value::List(_) => {}
                },
            }
        }
        if let Some(s) = root.class("Stage1") {
            m.nopx = s.text("texture").unwrap_or(FLAT_NOPX).to_string();
            if let Some(a) = s.class("uvTransform").map(|u| u.list("aside")) {
                m.tiling = a.first().and_then(|v| v.trim().parse().ok()).unwrap_or(m.tiling);
            }
        }
        if let Some(s) = root.class("Stage2") {
            m.texture = s.text("texture").unwrap_or("").to_string();
        }
        m
    }

    pub fn text(&self) -> String {
        let c = |v: [f32; 4]| v.iter().map(|x| num(*x)).collect::<Vec<_>>().join(",");
        let mut s = format!(
            "ambient[]={{{}}};\ndiffuse[]={{{}}};\nforcedDiffuse[]={{{}}};\nspecular[]={{{}}};\nspecularPower={};\nemmisive[]={{{}}};\nparallaxScale={};\n",
            c(self.ambient), c(self.diffuse), c(self.forced_diffuse), c(self.specular), num(self.specular_power), c(self.emmisive), num(self.parallax_scale)
        );
        for (k, v) in &self.other {
            let quoted = v.parse::<f64>().is_err();
            s += &if quoted { format!("{k}=\"{v}\";\n") } else { format!("{k}={v};\n") };
        }
        s += &format!("PixelShaderID=\"{}\";\nVertexShaderID=\"{}\";\n", self.pixel_shader, self.vertex_shader);
        let t = num(self.tiling);
        for (n, tex) in [(1, &self.nopx), (2, &self.texture)] {
            s += &format!("class Stage{n}\n{{\n\ttexture=\"{tex}\";\n\tuvSource=\"tex\";\n\tclass uvTransform\n\t{{\n\t\taside[]={{{t},0,0}};\n\t\tup[]={{0,{t},0}};\n\t\tdir[]={{0,0,0}};\n\t\tpos[]={{0,0,0}};\n\t}};\n}};\n");
        }
        s
    }

    /// Reads a material of the work drive.
    pub fn load(root: &Path, material: &str) -> Result<Self> {
        let path = root.join(material.replace('\\', "/"));
        let b = std::fs::read(&path).with_context(|| format!("{}", path.display()))?;
        Ok(Self::parse(&String::from_utf8_lossy(&b)))
    }
}

/// The relief a layer's material names, when that texture is on the work drive.
pub fn nopx_of_material(root: &Path, material: &str) -> Option<String> {
    let m = LayerMaterial::load(root, material).ok()?;
    let t = m.nopx.trim().replace('/', "\\");
    (!t.starts_with('#') && !t.is_empty() && root.join(t.replace('\\', "/")).is_file()).then_some(t)
}

// ---------------------------------------------------------------- textures

fn read_rgba(path: &Path) -> Result<(Vec<u8>, usize, usize)> {
    let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    if ext == "paa" {
        let t = forest_core::paa::decode(&std::fs::read(path)?, 8192)?;
        return Ok((t.rgba, t.width as usize, t.height as usize));
    }
    let img = image::open(path).with_context(|| format!("{}", path.display()))?.to_rgba8();
    let (w, h) = (img.width() as usize, img.height() as usize);
    Ok((img.into_raw(), w, h))
}

/// The nearest power of two, at most 4096.
fn pow2(n: usize) -> usize {
    let up = n.next_power_of_two();
    let down = (up / 2).max(1);
    (if up - n <= n - down { up } else { down }).clamp(4, 4096)
}

fn fit(rgba: Vec<u8>, w: usize, h: usize) -> (Vec<u8>, usize, usize) {
    let (tw, th) = (pow2(w), pow2(h));
    if (tw, th) == (w, h) {
        return (rgba, w, h);
    }
    let img = image::RgbaImage::from_raw(w as u32, h as u32, rgba).expect("size");
    let r = image::imageops::resize(&img, tw as u32, th as u32, image::imageops::FilterType::Triangle);
    (r.into_raw(), tw, th)
}

/// Writes a texture for the game at `target`: a PAA is copied, an image (PNG, TGA, JPG...) is
/// brought to sides of powers of two and compressed.
pub fn texture_into(source: &Path, target: &Path) -> Result<()> {
    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let ext = source.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    if ext == "paa" {
        if source.canonicalize().ok() != target.canonicalize().ok() {
            std::fs::copy(source, target)?;
        }
        return Ok(());
    }
    let (rgba, w, h) = read_rgba(source)?;
    let (rgba, w, h) = fit(rgba, w, h);
    crate::fsutil::atomic_write(target, &crate::paa_write::encode(&rgba, w, h, crate::paa_write::Format::Dxt5))
}

/// Makes a relief texture (`_nopx`) from a normal map (`_nohq` or an image) and, when given, a
/// height map (lighter is higher); without one the relief has no parallax.
pub fn make_nopx(normal: &Path, height: Option<&Path>, target: &Path) -> Result<()> {
    let (mut rgba, w, h) = read_rgba(normal)?;
    if rgba.len() != w * h * 4 || w == 0 {
        bail!("The normal map could not be read");
    }
    match height {
        Some(p) => {
            let (hm, hw, hh) = read_rgba(p)?;
            let img = image::RgbaImage::from_raw(hw as u32, hh as u32, hm).context("height map")?;
            let img = if (hw, hh) != (w, h) { image::imageops::resize(&img, w as u32, h as u32, image::imageops::FilterType::Triangle) } else { img };
            for (px, hp) in rgba.chunks_exact_mut(4).zip(img.pixels()) {
                let l = (hp[0] as u32 * 54 + hp[1] as u32 * 183 + hp[2] as u32 * 19) / 256;
                px[3] = l as u8;
            }
        }
        None => {
            for px in rgba.chunks_exact_mut(4) {
                px[3] = 128;
            }
        }
    }
    let (rgba, w, h) = fit(rgba, w, h);
    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir)?;
    }
    crate::fsutil::atomic_write(target, &crate::paa_write::encode(&rgba, w, h, crate::paa_write::Format::Dxt5))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_game_material_reads_and_writes_back() {
        let text = r#"ambient[] = {1, 1, 1, 1};
diffuse[] = {0.7, 0.7, 0.7, 1};
forcedDiffuse[] = {0, 0, 0, 0};
specular[] = {0, 0, 0, 0};
specularPower = 1;
emmisive[] = {0, 0, 0, 0};
parallaxScale = 0.75;
terrainIndirectLocalShadowBias = 0;
PixelShaderID = "NormalMapDiffuse";
VertexShaderID = "NormalMapDiffuseAlpha";
class Stage1 { texture = "dz\x\en_grass1_nopx.paa"; uvSource = "tex"; class uvTransform { aside[] = {12, 0, 0}; up[] = {0, 12, 0}; dir[] = {0, 0, 0}; pos[] = {0, 0, 0}; }; };
class Stage2 { texture = "dz\x\en_grass1_ca.paa"; uvSource = "tex"; class uvTransform { aside[] = {12, 0, 0}; up[] = {0, 12, 0}; dir[] = {0, 0, 0}; pos[] = {0, 0, 0}; }; };
"#;
        let m = LayerMaterial::parse(text);
        assert_eq!(m.nopx, "dz\\x\\en_grass1_nopx.paa");
        assert_eq!(m.texture, "dz\\x\\en_grass1_ca.paa");
        assert_eq!(m.tiling, 12.0);
        assert_eq!(m.diffuse, [0.7, 0.7, 0.7, 1.0]);
        assert_eq!(m.other, vec![("terrainIndirectLocalShadowBias".to_string(), "0".to_string())]);
        let again = LayerMaterial::parse(&m.text());
        assert_eq!(again, m);
        assert!(m.text().contains("aside[]={12,0,0};"));
    }

    #[test]
    fn images_become_textures_and_reliefs() {
        let d = tempfile::tempdir().unwrap();
        let png = d.path().join("n.png");
        image::RgbaImage::from_pixel(100, 60, image::Rgba([128, 128, 255, 255])).save(&png).unwrap();
        let hgt = d.path().join("h.png");
        image::RgbaImage::from_pixel(10, 10, image::Rgba([200, 200, 200, 255])).save(&hgt).unwrap();
        texture_into(&png, &d.path().join("out/a_ca.paa")).unwrap();
        let t = forest_core::paa::decode(&std::fs::read(d.path().join("out/a_ca.paa")).unwrap(), 4096).unwrap();
        assert_eq!((t.width, t.height), (128, 64));
        make_nopx(&png, Some(&hgt), &d.path().join("out/a_nopx.paa")).unwrap();
        let t = forest_core::paa::decode(&std::fs::read(d.path().join("out/a_nopx.paa")).unwrap(), 4096).unwrap();
        assert!((t.rgba[3] as i32 - 200).abs() < 8, "{}", t.rgba[3]);
    }
}
