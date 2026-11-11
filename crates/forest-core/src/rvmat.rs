//! RVMAT files, as text or binarised (raP), read into the same `Material` a model embeds.
use crate::odol::{render_flag, Material, Stage, TexGen};
use std::path::Path;

/// Pixel shaders of the engine, by their number in a binarised material.
pub mod shader {
    pub const NORMAL: u32 = 0;
    pub const NORMAL_DXTA: u32 = 1;
    pub const NORMAL_MAP: u32 = 2;
    pub const NORMAL_MAP_THROUGH: u32 = 3;
    pub const NORMAL_MAP_GRASS: u32 = 4;
    pub const NORMAL_MAP_DIFFUSE: u32 = 5;
    pub const DETAIL: u32 = 6;
    pub const WATER: u32 = 8;
    pub const WATER_SIMPLE: u32 = 9;
    pub const ALPHA_SHADOW: u32 = 12;
    pub const ALPHA_NO_SHADOW: u32 = 13;
    pub const DETAIL_MACRO_AS: u32 = 15;
    pub const NORMAL_MAP_MACRO_AS: u32 = 16;
    pub const NORMAL_MAP_DIFFUSE_MACRO_AS: u32 = 17;
    pub const NORMAL_MAP_SPECULAR_MAP: u32 = 18;
    pub const NORMAL_MAP_DETAIL_SPECULAR_MAP: u32 = 19;
    pub const NORMAL_MAP_MACRO_AS_SPECULAR_MAP: u32 = 20;
    pub const NORMAL_MAP_DETAIL_MACRO_AS_SPECULAR_MAP: u32 = 21;
    pub const NORMAL_MAP_SPECULAR_DI_MAP: u32 = 22;
    pub const NORMAL_MAP_DETAIL_SPECULAR_DI_MAP: u32 = 23;
    pub const NORMAL_MAP_MACRO_AS_SPECULAR_DI_MAP: u32 = 24;
    pub const NORMAL_MAP_DETAIL_MACRO_AS_SPECULAR_DI_MAP: u32 = 25;
    pub const TERRAIN_FIRST: u32 = 26;
    pub const TERRAIN_LAST: u32 = 55;
    pub const GLASS: u32 = 56;
    pub const NORMAL_MAP_SPECULAR_THROUGH: u32 = 58;
    pub const GRASS: u32 = 59;
    pub const NORMAL_MAP_THROUGH_SIMPLE: u32 = 60;
    pub const SUPER: u32 = 102;
    pub const MULTI: u32 = 103;
    pub const TERRAIN_X: u32 = 104;
    pub const TERRAIN_SIMPLE_X: u32 = 105;
    pub const TREE: u32 = 107;
    pub const TREE_PRT: u32 = 108;
    pub const TREE_SIMPLE: u32 = 109;
    pub const SKIN: u32 = 110;
    pub const CALM_WATER: u32 = 111;
    pub const TREE_ATOC: u32 = 112;
    pub const GRASS_ATOC: u32 = 113;
    pub const TREE_ADV: u32 = 114;
    pub const TREE_ADV_SIMPLE: u32 = 115;
    pub const TREE_ADV_TRUNK: u32 = 116;
    pub const TREE_ADV_TRUNK_SIMPLE: u32 = 117;
    pub const TREE_ADV_ATOC: u32 = 118;
    pub const TREE_ADV_SIMPLE_ATOC: u32 = 119;
    pub const TREE_SN: u32 = 120;
    pub const TERRAIN_SN_X: u32 = 122;
    pub const SUPER_HAIR: u32 = 129;
    /// A name the table below does not know; its stages are told apart by their textures.
    pub const UNKNOWN: u32 = u32::MAX;
}

const SHADER_NAMES: &[(&str, u32)] = &[
    ("normal", shader::NORMAL),
    ("normaldxta", shader::NORMAL_DXTA),
    ("normalmap", shader::NORMAL_MAP),
    ("normalmapthrough", shader::NORMAL_MAP_THROUGH),
    ("normalmapgrass", shader::NORMAL_MAP_GRASS),
    ("normalmapdiffuse", shader::NORMAL_MAP_DIFFUSE),
    ("detail", shader::DETAIL),
    ("interpolation", 7),
    ("water", shader::WATER),
    ("watersimple", shader::WATER_SIMPLE),
    ("white", 10),
    ("whitealpha", 11),
    ("alphashadow", shader::ALPHA_SHADOW),
    ("alphanoshadow", shader::ALPHA_NO_SHADOW),
    ("detailmacroas", shader::DETAIL_MACRO_AS),
    ("normalmapmacroas", shader::NORMAL_MAP_MACRO_AS),
    ("normalmapdiffusemacroas", shader::NORMAL_MAP_DIFFUSE_MACRO_AS),
    ("normalmapspecularmap", shader::NORMAL_MAP_SPECULAR_MAP),
    ("normalmapdetailspecularmap", shader::NORMAL_MAP_DETAIL_SPECULAR_MAP),
    ("normalmapmacroasspecularmap", shader::NORMAL_MAP_MACRO_AS_SPECULAR_MAP),
    ("normalmapdetailmacroasspecularmap", shader::NORMAL_MAP_DETAIL_MACRO_AS_SPECULAR_MAP),
    ("normalmapspeculardimap", shader::NORMAL_MAP_SPECULAR_DI_MAP),
    ("normalmapdetailspeculardimap", shader::NORMAL_MAP_DETAIL_SPECULAR_DI_MAP),
    ("normalmapmacroasspeculardimap", shader::NORMAL_MAP_MACRO_AS_SPECULAR_DI_MAP),
    ("normalmapdetailmacroasspeculardimap", shader::NORMAL_MAP_DETAIL_MACRO_AS_SPECULAR_DI_MAP),
    ("glass", shader::GLASS),
    ("nontl", 57),
    ("normalmapspecularthrough", shader::NORMAL_MAP_SPECULAR_THROUGH),
    ("grass", shader::GRASS),
    ("normalmapthroughsimple", shader::NORMAL_MAP_THROUGH_SIMPLE),
    ("super", shader::SUPER),
    ("superext", shader::SUPER),
    ("multi", shader::MULTI),
    ("terrainx", shader::TERRAIN_X),
    ("terrainsimplex", shader::TERRAIN_SIMPLE_X),
    ("tree", shader::TREE),
    ("treeprt", shader::TREE_PRT),
    ("treesimple", shader::TREE_SIMPLE),
    ("skin", shader::SKIN),
    ("calmwater", shader::CALM_WATER),
    ("treeatoc", shader::TREE_ATOC),
    ("grassatoc", shader::GRASS_ATOC),
    ("treeadv", shader::TREE_ADV),
    ("treeadvsimple", shader::TREE_ADV_SIMPLE),
    ("treeadvtrunk", shader::TREE_ADV_TRUNK),
    ("treeadvtrunksimple", shader::TREE_ADV_TRUNK_SIMPLE),
    ("treeadvatoc", shader::TREE_ADV_ATOC),
    ("treeadvsimpleatoc", shader::TREE_ADV_SIMPLE_ATOC),
    ("treesn", shader::TREE_SN),
    ("terrainsnx", shader::TERRAIN_SN_X),
    ("superhair", shader::SUPER_HAIR),
];

/// Engine number of a `PixelShaderID`; a missing one is the plain `Normal` shader.
pub fn pixel_shader_id(name: &str) -> u32 {
    let n = name.trim().to_ascii_lowercase();
    if n.is_empty() {
        return shader::NORMAL;
    }
    if let Some(rest) = n.strip_prefix("terrain") {
        if let Ok(k) = rest.parse::<u32>() {
            return (shader::TERRAIN_FIRST + k.saturating_sub(1)).min(shader::TERRAIN_LAST);
        }
    }
    SHADER_NAMES.iter().find(|(k, _)| *k == n).map(|(_, v)| *v).unwrap_or(shader::UNKNOWN)
}

fn render_flag_bit(name: &str) -> u32 {
    match name.trim().to_ascii_lowercase().as_str() {
        "alwaysinshadow" => render_flag::ALWAYS_IN_SHADOW,
        "nozwrite" => render_flag::NO_Z_WRITE,
        "nocolorwrite" => render_flag::NO_COLOR_WRITE,
        "noalphawrite" => render_flag::NO_ALPHA_WRITE,
        "addblend" => render_flag::ADD_BLEND,
        "alphatest32" => render_flag::ALPHA_TEST_32,
        "alphatest64" => render_flag::ALPHA_TEST_64,
        "alphatest128" | "alphatest256" => render_flag::ALPHA_TEST_128,
        _ => 0,
    }
}

/// A value of a config entry.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Text(String),
    Number(f32),
    List(Vec<Value>),
}
impl Value {
    fn number(&self) -> Option<f32> {
        match self {
            Value::Number(v) => Some(*v),
            Value::Text(t) => t.trim().parse().ok(),
            Value::List(_) => None,
        }
    }
    fn text(&self) -> String {
        match self {
            Value::Text(t) => t.clone(),
            Value::Number(v) => v.to_string(),
            Value::List(_) => String::new(),
        }
    }
    fn numbers(&self) -> Vec<f32> {
        match self {
            Value::List(l) => l.iter().filter_map(Value::number).collect(),
            _ => Vec::new(),
        }
    }
}

/// A config class: its entries and nested classes, names in lower case.
#[derive(Debug, Clone, Default)]
pub struct Class {
    /// Class this one inherits from, empty when it has none.
    pub parent: String,
    pub values: Vec<(String, Value)>,
    pub classes: Vec<(String, Class)>,
}
impl Class {
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.values.iter().find(|(k, _)| k == name).map(|(_, v)| v)
    }
    pub fn class(&self, name: &str) -> Option<&Class> {
        self.classes.iter().find(|(k, _)| k == name).map(|(_, c)| c)
    }
}

struct Text<'a> {
    s: &'a [u8],
    p: usize,
}
impl Text<'_> {
    fn skip_line(&mut self) {
        while self.s.get(self.p).is_some_and(|c| *c != b'\n') {
            self.p += 1;
        }
    }
    fn space(&mut self) {
        loop {
            while self.s.get(self.p).is_some_and(|c| c.is_ascii_whitespace()) {
                self.p += 1;
            }
            match (self.s.get(self.p), self.s.get(self.p + 1)) {
                (Some(b'/'), Some(b'/')) => self.skip_line(),
                (Some(b'/'), Some(b'*')) => {
                    self.p += 2;
                    while self.p < self.s.len() && !self.s[self.p..].starts_with(b"*/") {
                        self.p += 1;
                    }
                    self.p += 2;
                }
                _ => return,
            }
        }
    }
    fn peek(&mut self) -> Option<u8> {
        self.space();
        self.s.get(self.p).copied()
    }
    fn eat(&mut self, c: u8) -> bool {
        if self.peek() == Some(c) {
            self.p += 1;
            true
        } else {
            false
        }
    }
    fn word(&mut self) -> String {
        self.space();
        let start = self.p;
        while self.s.get(self.p).is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_') {
            self.p += 1;
        }
        String::from_utf8_lossy(&self.s[start..self.p]).to_ascii_lowercase()
    }
    fn value(&mut self, depth: usize) -> Value {
        match self.peek() {
            Some(b'{') if depth < 16 => {
                self.p += 1;
                let mut list = Vec::new();
                while !matches!(self.peek(), Some(b'}') | None) {
                    let before = self.p;
                    list.push(self.value(depth + 1));
                    self.eat(b',');
                    if self.p == before {
                        self.p += 1;
                    }
                }
                self.eat(b'}');
                Value::List(list)
            }
            Some(b'"') => {
                self.p += 1;
                let mut out = Vec::new();
                while let Some(&c) = self.s.get(self.p) {
                    self.p += 1;
                    if c == b'"' {
                        // A doubled quote stands for one.
                        if self.s.get(self.p) == Some(&b'"') {
                            self.p += 1;
                        } else {
                            break;
                        }
                    }
                    out.push(c);
                }
                Value::Text(String::from_utf8_lossy(&out).into_owned())
            }
            _ => {
                let start = self.p;
                while self.s.get(self.p).is_some_and(|c| !matches!(c, b';' | b',' | b'}' | b'\n' | b'\r')) {
                    self.p += 1;
                }
                let raw = String::from_utf8_lossy(&self.s[start..self.p]).trim().to_string();
                raw.parse::<f32>().map(Value::Number).unwrap_or(Value::Text(raw))
            }
        }
    }
    fn body(&mut self, depth: usize) -> Class {
        let mut out = Class::default();
        while let Some(c) = self.peek() {
            if c == b'}' {
                break;
            }
            let before = self.p;
            if c == b'#' {
                // Preprocessor lines carry nothing a material needs.
                self.skip_line();
                continue;
            }
            let name = self.word();
            if name == "class" && depth < 16 {
                let name = self.word();
                let parent = if self.eat(b':') { self.word() } else { String::new() };
                if self.eat(b'{') {
                    let mut inner = self.body(depth + 1);
                    inner.parent = parent;
                    self.eat(b'}');
                    out.classes.push((name, inner));
                }
            } else if !name.is_empty() {
                if self.eat(b'[') {
                    self.eat(b']');
                }
                if self.eat(b'=') {
                    let v = self.value(0);
                    out.values.push((name, v));
                }
            }
            self.eat(b';');
            if self.p == before {
                self.p += 1;
            }
        }
        out
    }
}

/// Parses the text form of a config.
pub fn parse_text(text: &[u8]) -> Class {
    Text { s: text, p: 0 }.body(0)
}

struct Bin<'a> {
    b: &'a [u8],
    p: usize,
}
impl Bin<'_> {
    fn u8(&mut self) -> Option<u8> {
        let v = *self.b.get(self.p)?;
        self.p += 1;
        Some(v)
    }
    fn u32(&mut self) -> Option<u32> {
        let v = u32::from_le_bytes(self.b.get(self.p..self.p + 4)?.try_into().ok()?);
        self.p += 4;
        Some(v)
    }
    /// Counts are stored seven bits at a time.
    fn count(&mut self) -> Option<usize> {
        let (mut v, mut shift) = (0usize, 0);
        loop {
            let c = self.u8()? as usize;
            v |= (c & 0x7f) << shift;
            shift += 7;
            if c & 0x80 == 0 || shift > 28 {
                return Some(v);
            }
        }
    }
    fn string(&mut self) -> Option<String> {
        let rest = self.b.get(self.p..)?;
        let end = rest.iter().position(|c| *c == 0)?;
        self.p += end + 1;
        Some(String::from_utf8_lossy(&rest[..end]).into_owned())
    }
    fn scalar(&mut self, kind: u8, depth: usize) -> Option<Value> {
        Some(match kind {
            0 | 4 => Value::Text(self.string()?),
            1 => Value::Number(f32::from_bits(self.u32()?)),
            2 => Value::Number(self.u32()? as i32 as f32),
            3 if depth < 16 => self.list(depth + 1)?,
            6 => {
                let lo = self.u32()? as u64;
                Value::Number((lo | (self.u32()? as u64) << 32) as i64 as f32)
            }
            _ => return None,
        })
    }
    fn list(&mut self, depth: usize) -> Option<Value> {
        let n = self.count()?;
        let mut out = Vec::with_capacity(n.min(4096));
        for _ in 0..n {
            let kind = self.u8()?;
            out.push(self.scalar(kind, depth)?);
        }
        Some(Value::List(out))
    }
    fn body(&mut self, at: usize, depth: usize) -> Option<Class> {
        if depth > 16 {
            return None;
        }
        self.p = at;
        let parent = self.string()?.to_ascii_lowercase();
        let n = self.count()?;
        let mut out = Class { parent, ..Default::default() };
        let mut nested = Vec::new();
        for _ in 0..n {
            match self.u8()? {
                0 => {
                    let name = self.string()?.to_ascii_lowercase();
                    nested.push((name, self.u32()? as usize));
                }
                1 => {
                    let kind = self.u8()?;
                    let name = self.string()?.to_ascii_lowercase();
                    let v = self.scalar(kind, depth)?;
                    out.values.push((name, v));
                }
                2 => {
                    let name = self.string()?.to_ascii_lowercase();
                    let v = self.list(depth)?;
                    out.values.push((name, v));
                }
                3 | 4 => {
                    self.string()?;
                }
                5 => {
                    self.u32()?;
                    let name = self.string()?.to_ascii_lowercase();
                    let v = self.list(depth)?;
                    out.values.push((name, v));
                }
                _ => return None,
            }
        }
        for (name, offset) in nested {
            let inner = self.body(offset, depth + 1)?;
            out.classes.push((name, inner));
        }
        Some(out)
    }
}

/// Parses a config in either form.
pub fn parse(bytes: &[u8]) -> Option<Class> {
    if bytes.starts_with(b"\0raP") {
        Bin { b: bytes, p: 0 }.body(16, 0)
    } else {
        Some(parse_text(bytes))
    }
}

fn color(c: &Class, name: &str, default: [f32; 4]) -> [f32; 4] {
    let v = c.get(name).map(Value::numbers).unwrap_or_default();
    std::array::from_fn(|i| v.get(i).copied().unwrap_or(default[i]))
}

const IDENTITY: [f32; 12] = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0];

/// The class itself, or the one it inherits its coordinates from (`class TexGen5 : TexGen0 {}`).
fn inherited<'a>(root: &'a Class, mut c: &'a Class) -> &'a Class {
    for _ in 0..8 {
        if c.get("uvsource").is_some() || c.class("uvtransform").is_some() || c.parent.is_empty() {
            break;
        }
        match root.class(&c.parent) {
            Some(p) => c = p,
            None => break,
        }
    }
    c
}

fn tex_gen(c: &Class) -> TexGen {
    let source = match c.get("uvsource").map(Value::text).unwrap_or_default().to_ascii_lowercase().as_str() {
        "tex1" => 5,
        // The binariser stores "none" as the first set too.
        _ => 1,
    };
    let mut matrix = IDENTITY;
    if let Some(t) = c.class("uvtransform") {
        for (k, name) in ["aside", "up", "dir", "pos"].iter().enumerate() {
            if let Some(v) = t.get(name).map(Value::numbers).filter(|v| v.len() >= 3) {
                matrix[k * 3..k * 3 + 3].copy_from_slice(&v[..3]);
            }
        }
    }
    TexGen { source, matrix }
}

/// Builds the material a parsed RVMAT describes.
pub fn material(name: &str, c: &Class) -> Material {
    let base = Material::default();
    let mut m = Material {
        name: name.to_string(),
        pixel_shader: pixel_shader_id(&c.get("pixelshaderid").map(Value::text).unwrap_or_default()),
        emissive: color(c, "emmisive", base.emissive),
        ambient: color(c, "ambient", base.ambient),
        diffuse: color(c, "diffuse", base.diffuse),
        forced_diffuse: color(c, "forceddiffuse", base.forced_diffuse),
        specular: color(c, "specular", base.specular),
        specular_power: c.get("specularpower").and_then(Value::number).unwrap_or(base.specular_power),
        ..base
    };
    if let Some(Value::List(flags)) = c.get("renderflags") {
        m.render_flags = flags.iter().map(|f| render_flag_bit(&f.text())).fold(0, |a, b| a | b);
    }
    let number = |k: &str| c.get(k).and_then(Value::number);
    if let Some(layer) = number("multitopprojectionlayer").filter(|l| *l >= 0.0) {
        m.top_projection = Some((layer as u32, number("degangletopprojectionstart").unwrap_or(30.0), number("degangletopprojectionend").unwrap_or(45.0)));
    }
    // Generator 0 leaves the coordinates as they are.
    m.transforms.push(TexGen { source: 1, matrix: IDENTITY });
    let mut shared = std::collections::HashMap::new();
    for (k, class) in &c.classes {
        if let Some(i) = k.strip_prefix("texgen").and_then(|n| n.parse::<u32>().ok()) {
            m.transforms.push(tex_gen(inherited(c, class)));
            shared.insert(i, m.transforms.len() as u32 - 1);
        }
    }
    for (k, class) in &c.classes {
        let Some(i) = k.strip_prefix("stage").and_then(|n| n.parse::<usize>().ok()).filter(|i| *i < 32) else { continue };
        if m.stages.len() <= i {
            m.stages.resize(i + 1, Stage::default());
        }
        let transform = match class.get("texgen").and_then(Value::number).and_then(|g| shared.get(&(g as u32))) {
            Some(t) => *t,
            None => {
                m.transforms.push(tex_gen(class));
                m.transforms.len() as u32 - 1
            }
        };
        m.stages[i] = Stage { texture: class.get("texture").map(Value::text).unwrap_or_default(), transform };
    }
    if m.stages.is_empty() {
        m.stages.push(Stage::default());
    }
    m
}

/// Reads the RVMAT `name` (a path relative to the work drive) under `root`.
pub fn load(root: &Path, name: &str) -> Option<Material> {
    let rel = name.trim().trim_start_matches(['\\', '/']);
    if rel.is_empty() {
        return None;
    }
    let bytes = std::fs::read(root.join(rel.replace('\\', "/"))).ok()?;
    Some(material(rel, &parse(&bytes)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SUPER: &str = r##"
// comment
ambient[] = {1, 1, 1, 1};
diffuse[]={0.5,0.6,0.7,1};
forcedDiffuse[] = {0, 0, 0, 0};
emmisive[] = {0, 0, 0, 1};
specular[] = {0.2, 0.2, 0.2, 1};
specularPower = 40; /* block */
renderFlags[] = {"NoZWrite", "AlphaTest64"};
PixelShaderID = "Super";
VertexShaderID = "Super";
class Stage1
{
	texture = "dz\a\data\a_nohq.paa";
	uvSource = "tex";
	class uvTransform
	{
		aside[] = {2, 0, 0};
		up[] = {0, 3, 0};
		dir[] = {0, 0, 0};
		pos[] = {0.25, 0.5, 0};
	};
};
class Stage5
{
	texture = "#(argb,8,8,3)color(1,0.2,0.4,1,SMDI)";
	uvSource = "tex1";
};
"##;

    #[test]
    fn text_material_keeps_colours_flags_and_stages() {
        let m = material("a.rvmat", &parse(SUPER.as_bytes()).unwrap());
        assert_eq!(m.pixel_shader, shader::SUPER);
        assert_eq!(m.diffuse, [0.5, 0.6, 0.7, 1.0]);
        assert_eq!(m.specular_power, 40.0);
        assert_eq!(m.render_flags, render_flag::NO_Z_WRITE | render_flag::ALPHA_TEST_64);
        assert_eq!(m.stages.len(), 6);
        assert!(m.stages[0].texture.is_empty() && m.stages[2].texture.is_empty());
        assert!(m.stages[1].texture.ends_with("a_nohq.paa"));
        assert!(m.stages[5].texture.starts_with("#(argb"));
        let t = &m.transforms[m.stages[1].transform as usize];
        assert_eq!((t.source, t.matrix[0], t.matrix[4], t.matrix[9], t.matrix[10]), (1, 2.0, 3.0, 0.25, 0.5));
        assert_eq!(m.transforms[m.stages[5].transform as usize].source, 5);
    }

    #[test]
    fn shared_generators_and_top_projection() {
        let text = r#"multiTopProjectionLayer = 3; degAngleTopProjectionStart = 50; degAngleTopProjectionEnd = 70;
PixelShaderID = "Multi";
class TexGen0 { uvSource = "tex"; class uvTransform { aside[] = {4,0,0}; up[] = {0,4,0}; dir[] = {0,0,0}; pos[] = {0,0,0}; }; };
class TexGen4 { uvSource = "tex1"; };
class Stage0 { texture = "a_co.paa"; texGen = "0"; };
class Stage4 { texture = "a_mask.paa"; texGen = 4; };"#;
        let m = material("m.rvmat", &parse(text.as_bytes()).unwrap());
        assert_eq!(m.pixel_shader, shader::MULTI);
        assert_eq!(m.top_projection, Some((3, 50.0, 70.0)));
        assert_eq!(m.transforms[m.stages[0].transform as usize].matrix[0], 4.0);
        assert_eq!(m.transforms[m.stages[4].transform as usize].source, 5);
    }

    #[test]
    fn generators_inherit_their_coordinates() {
        let text = r#"PixelShaderID = "Multi";
class TexGen0 { uvSource = "tex1"; class uvTransform { aside[] = {0,-12,0}; up[] = {16,0,0}; dir[] = {0,0,0}; pos[] = {0,0,0}; }; };
class TexGen5 : TexGen0 {};
class Stage5 { texture = "a_dtsmdi.paa"; texGen = "5"; };"#;
        let m = material("m.rvmat", &parse(text.as_bytes()).unwrap());
        let t = &m.transforms[m.stages[5].transform as usize];
        assert_eq!((t.source, t.matrix[1], t.matrix[3]), (5, -12.0, 16.0));
    }

    #[test]
    fn shader_names_map_to_engine_numbers() {
        assert_eq!(pixel_shader_id("NormalMapDetailMacroASSpecularDIMap"), 25);
        assert_eq!(pixel_shader_id("TreeAdvTrunk"), 116);
        assert_eq!(pixel_shader_id(""), shader::NORMAL);
        assert_eq!(pixel_shader_id("Terrain3"), 28);
        assert_eq!(pixel_shader_id("SomethingNew"), shader::UNKNOWN);
    }

    #[test]
    fn binarised_config_reads_like_text() {
        // Root class: one value, one array and a nested class.
        let mut b = b"\0raP".to_vec();
        b.extend_from_slice(&[0, 0, 0, 0, 8, 0, 0, 0, 0, 0, 0, 0]);
        b.push(0);
        b.push(3);
        b.extend_from_slice(b"\x01\x00PixelShaderID\0Glass\0");
        b.extend_from_slice(b"\x02diffuse\0\x02\x01");
        b.extend_from_slice(&0.5f32.to_le_bytes());
        b.push(2);
        b.extend_from_slice(&2u32.to_le_bytes());
        b.extend_from_slice(b"\x00Stage1\0");
        let at = b.len() + 4;
        b.extend_from_slice(&(at as u32).to_le_bytes());
        b.extend_from_slice(b"\0\x01\x01\x00texture\0x_co.paa\0");
        let m = material("g.rvmat", &parse(&b).unwrap());
        assert_eq!(m.pixel_shader, shader::GLASS);
        assert_eq!(&m.diffuse[..2], &[0.5, 2.0]);
        assert_eq!(m.stages[1].texture, "x_co.paa");
    }

    /// Materials of the game parse into known shaders when P: is mounted.
    #[test]
    fn game_materials_have_known_shaders() {
        let root = Path::new("P:/");
        for name in ["dz\\rocks\\data\\rock_apart1\\rock_apart1.rvmat", "dz\\plants\\tree\\data\\b_betulapendula_1f_leaves.rvmat", "dz\\data\\data\\chrome_enviro.rvmat"] {
            let Some(m) = load(root, name) else { return };
            assert_ne!(m.pixel_shader, shader::UNKNOWN, "{name}");
            assert!(!m.stages.is_empty());
        }
    }
}
