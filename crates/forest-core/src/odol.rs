//! Minimal reader for binarised (ODOL) P3D models: LOD resolutions, vertex positions and
//! named selections. Enough to measure track models shipped only as ODOL (DayZ 1.30 is
//! version 56); materials, normals and animations are skipped.
use anyhow::{Context, Result, bail, ensure};

/// One LOD: its resolution, vertex positions (model x, y, z) and named selections with
/// their vertex indices.
#[derive(Debug, Default)]
pub struct Lod {
    pub resolution: f32,
    pub vertices: Vec<[f32; 3]>,
    pub selections: Vec<(String, Vec<u16>)>,
    /// Texture paths of the LOD.
    pub textures: Vec<String>,
    /// Polygons as vertex indices (3 or 4).
    pub faces: Vec<Vec<u16>>,
    /// Texture index of each face (`u16::MAX` when none).
    pub face_textures: Vec<u16>,
    /// Material of each face (-1 when none).
    pub face_materials: Vec<i32>,
    pub materials: Vec<Material>,
    /// First UV set, one per vertex.
    pub uvs: Vec<[f32; 2]>,
    /// Second UV set, empty when the LOD has only one.
    pub uvs2: Vec<[f32; 2]>,
    /// Normal of each vertex as the model stores it; empty when it could not be read.
    /// Its sign is not settled here: compare with the faces before use.
    pub normals: Vec<[f32; 3]>,
}
impl Lod {
    /// First vertex of a named selection (case-insensitive).
    pub fn point(&self, name: &str) -> Option<[f32; 3]> {
        self.selections
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .and_then(|(_, v)| v.first())
            .and_then(|&i| self.vertices.get(i as usize).copied())
    }
}
/// Resolution of the memory LOD.
pub const MEMORY: f32 = 1e15;

struct Reader<'a> {
    b: &'a [u8],
    p: usize,
}
impl<'a> Reader<'a> {
    fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.p.checked_add(n).context("Rozmiar ODOL")?;
        let s = self.b.get(self.p..end).context("Niepełny plik ODOL")?;
        self.p = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.bytes(1)?[0])
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.bytes(4)?.try_into()?))
    }
    fn i32(&mut self) -> Result<i32> {
        Ok(self.u32()? as i32)
    }
    fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_le_bytes(self.bytes(4)?.try_into()?))
    }
    fn count(&mut self) -> Result<usize> {
        let n = self.i32()?;
        ensure!((0..=50_000_000).contains(&n), "Nieprawidłowa liczba elementów ODOL");
        Ok(n as usize)
    }
    fn skip(&mut self, n: usize) -> Result<()> {
        self.bytes(n).map(|_| ())
    }
    fn cstring(&mut self) -> Result<String> {
        let rest = self.b.get(self.p..).context("Niepełny plik ODOL")?;
        let end = rest.iter().position(|&c| c == 0).context("Niezakończony napis ODOL")?;
        let s = String::from_utf8_lossy(&rest[..end]).into_owned();
        self.p += end + 1;
        Ok(s)
    }
    /// `size` bytes, LZO-compressed when 1024 bytes or more.
    fn compressed(&mut self, size: usize) -> Result<Vec<u8>> {
        if size < 1024 {
            return Ok(self.bytes(size)?.to_vec());
        }
        let rest = self.b.get(self.p..).context("Niepełny plik ODOL")?;
        let (data, used) = lzo1x_decompress(rest, size)?;
        self.p += used;
        Ok(data)
    }
    /// Compressed array: element count, then `count × element` bytes.
    fn compressed_array(&mut self, element: usize) -> Result<(usize, Vec<u8>)> {
        let n = self.count()?;
        Ok((n, self.compressed(n.checked_mul(element).context("Rozmiar ODOL")?)?))
    }
    /// Compressed array that may instead store one value repeated `count` times.
    fn compressed_fill(&mut self, element: usize) -> Result<()> {
        let n = self.count()?;
        if self.u8()? != 0 {
            self.skip(element)
        } else {
            self.compressed(n.checked_mul(element).context("Rozmiar ODOL")?).map(|_| ())
        }
    }
}

/// LZO1X decompression of exactly `expected` bytes; returns the data and input consumed.
pub(crate) fn lzo1x_decompress(input: &[u8], expected: usize) -> Result<(Vec<u8>, usize)> {
    let mut out: Vec<u8> = Vec::with_capacity(expected);
    let mut ip = 0usize;
    let byte = |ip: &mut usize| -> Result<usize> {
        let b = *input.get(*ip).context("Uszkodzone dane LZO")?;
        *ip += 1;
        Ok(b as usize)
    };
    let copy_literal = |out: &mut Vec<u8>, ip: &mut usize, n: usize| -> Result<()> {
        let s = input.get(*ip..*ip + n).context("Uszkodzone dane LZO")?;
        out.extend_from_slice(s);
        *ip += n;
        Ok(())
    };
    let copy_match = |out: &mut Vec<u8>, distance: usize, n: usize| -> Result<()> {
        ensure!(distance >= 1 && distance <= out.len(), "Uszkodzone dane LZO");
        let start = out.len() - distance;
        for i in 0..n {
            let b = out[start + i];
            out.push(b);
        }
        Ok(())
    };
    // Number of literals copied after the previous instruction (0–3), from its low bits.
    let mut state;
    let first = *input.first().context("Puste dane LZO")? as usize;
    if first > 17 {
        ip += 1;
        let n = first - 17;
        copy_literal(&mut out, &mut ip, n)?;
        state = if n < 4 { n } else { 4 };
    } else {
        state = 0;
    }
    loop {
        ensure!(out.len() <= expected, "Uszkodzone dane LZO");
        let t = byte(&mut ip)?;
        if t < 16 {
            if state == 0 {
                // Literal run.
                let mut n = t;
                if n == 0 {
                    n = 15;
                    loop {
                        let b = byte(&mut ip)?;
                        if b != 0 {
                            n += b;
                            break;
                        }
                        n += 255;
                    }
                }
                copy_literal(&mut out, &mut ip, n + 3)?;
                state = 4;
                continue;
            }
            let b = byte(&mut ip)?;
            if state == 4 {
                // Three-byte match at distance 2049–3072 after a long literal run.
                copy_match(&mut out, 1 + 0x0800 + (t >> 2) + (b << 2), 3)?;
            } else {
                // Two-byte match after a short literal run.
                copy_match(&mut out, 1 + (t >> 2) + (b << 2), 2)?;
            }
            state = t & 3;
        } else if t >= 64 {
            let b = byte(&mut ip)?;
            copy_match(&mut out, 1 + ((t >> 2) & 7) + (b << 3), (t >> 5) + 1)?;
            state = t & 3;
        } else if t >= 32 {
            let mut n = t & 31;
            if n == 0 {
                n = 31;
                loop {
                    let b = byte(&mut ip)?;
                    if b != 0 {
                        n += b;
                        break;
                    }
                    n += 255;
                }
            }
            let (b0, b1) = (byte(&mut ip)?, byte(&mut ip)?);
            copy_match(&mut out, 1 + (b0 >> 2) + (b1 << 6), n + 2)?;
            state = b0 & 3;
        } else {
            // 16–31: far match or end of stream.
            let mut n = t & 7;
            if n == 0 {
                n = 7;
                loop {
                    let b = byte(&mut ip)?;
                    if b != 0 {
                        n += b;
                        break;
                    }
                    n += 255;
                }
            }
            let (b0, b1) = (byte(&mut ip)?, byte(&mut ip)?);
            let distance = ((t & 8) << 11) + (b0 >> 2) + (b1 << 6);
            if distance == 0 {
                break;
            }
            copy_match(&mut out, distance + 0x4000, n + 2)?;
            state = b0 & 3;
        }
        if state > 0 && state < 4 {
            copy_literal(&mut out, &mut ip, state)?;
        }
    }
    ensure!(out.len() == expected, "Rozmiar danych LZO nie zgadza się z ODOL");
    Ok((out, ip))
}

/// LOD resolutions and byte ranges. The table follows the variable-length model info, so it
/// is located by its own consistency: start/end offsets, flags and metadata must lead exactly
/// to the first LOD.
/// ODOL version and (resolution, start, end) of every LOD.
type LodTable = (u32, Vec<(f32, usize, usize)>);
fn lod_table(b: &[u8]) -> Result<LodTable> {
    ensure!(b.starts_with(b"ODOL"), "To nie jest model ODOL");
    let mut r = Reader { b, p: 4 };
    let version = r.u32()?;
    ensure!((53..=60).contains(&version), "Nieobsługiwana wersja ODOL {version}");
    let n = r.count()?;
    ensure!((1..=64).contains(&n), "Nieprawidłowa liczba LOD w ODOL");
    let resolutions: Vec<f32> = (0..n).map(|_| r.f32()).collect::<Result<_>>()?;
    let word = |p: usize| b.get(p..p + 4).map(|w| u32::from_le_bytes(w.try_into().unwrap()) as usize);
    'scan: for p in r.p..b.len().saturating_sub(n * 9) {
        let mut ranges = Vec::with_capacity(n);
        for i in 0..n {
            let (Some(s), Some(e)) = (word(p + i * 4), word(p + (n + i) * 4)) else {
                continue 'scan;
            };
            if s <= p || e <= s || e > b.len() {
                continue 'scan;
            }
            ranges.push((s, e));
        }
        let mut q = p + n * 8;
        let mut meta = 0;
        for i in 0..n {
            match b.get(q + i) {
                Some(0) => meta += 1,
                Some(1) => {}
                _ => continue 'scan,
            }
        }
        q += n + meta * 25;
        if ranges.iter().map(|r| r.0).min() != Some(q) {
            continue;
        }
        let mut sorted = ranges.clone();
        sorted.sort();
        if sorted.windows(2).any(|w| w[0].1 > w[1].0) {
            continue;
        }
        return Ok((
            version,
            resolutions.into_iter().zip(ranges).map(|(res, (s, e))| (res, s, e)).collect(),
        ));
    }
    bail!("Nie znaleziono tabeli LOD w ODOL")
}

fn read_lod(b: &[u8], start: usize, resolution: f32) -> Result<Lod> {
    let mut r = Reader { b, p: start };
    for _ in 0..r.count()? {
        // Proxy: model name, 4×3 matrix, four indices.
        r.cstring()?;
        r.skip(48 + 16)?;
    }
    let n = r.count()?;
    r.skip(n * 4)?;
    for _ in 0..r.count()? {
        let n = r.count()?;
        r.skip(n * 4)?;
    }
    // Vertex count, face area, hints, bounding box, centre and radius.
    r.skip(4 + 4 + 8 + 36 + 4)?;
    let mut textures = Vec::new();
    for _ in 0..r.count()? {
        textures.push(r.cstring()?);
    }
    let mut materials = Vec::new();
    for _ in 0..r.count()? {
        materials.push(read_material(&mut r)?);
    }
    r.compressed_array(2)?;
    r.compressed_array(2)?;
    // Faces: count, allocation size, reserved word, then (vertex count, indices).
    let count = r.count()?;
    r.skip(4 + 2)?;
    let mut faces = Vec::with_capacity(count);
    for _ in 0..count {
        let k = r.u8()? as usize;
        let data = r.bytes(k * 2)?;
        faces.push((0..k).map(|i| u16::from_le_bytes([data[i * 2], data[i * 2 + 1]])).collect::<Vec<_>>());
    }
    // Sections give face ranges as byte offsets: 8 per triangle, 10 per quad.
    let mut offsets = Vec::with_capacity(faces.len());
    let mut offset = 0;
    for f in &faces {
        offsets.push(offset);
        offset += if f.len() == 4 { 10 } else { 8 };
    }
    let mut face_textures = vec![u16::MAX; faces.len()];
    let mut face_materials = vec![-1i32; faces.len()];
    for _ in 0..r.count()? {
        let lower = r.i32()?;
        let upper = r.i32()?;
        r.skip(12)?;
        let texture = u16::from_le_bytes(r.bytes(2)?.try_into().unwrap());
        r.skip(4)?;
        let material = r.i32()?;
        if material == -1 {
            r.cstring()?;
        }
        let n = r.count()?;
        r.skip(n * 4)?;
        // Surfaces without a texture take the colour map of their material.
        let mut texture = texture;
        if texture == u16::MAX && material >= 0 {
            if let Some(name) = materials.get(material as usize).and_then(|m: &Material| diffuse_stage(&m.stages.iter().map(|s| s.texture.clone()).collect::<Vec<_>>())) {
                let slot = textures.iter().position(|t| *t == name).unwrap_or_else(|| {
                    textures.push(name);
                    textures.len() - 1
                });
                texture = slot as u16;
            }
        }
        for (i, &o) in offsets.iter().enumerate() {
            if o >= lower && o < upper {
                face_textures[i] = texture;
                face_materials[i] = material;
            }
        }
    }
    let mut selections = Vec::new();
    for _ in 0..r.count()? {
        let name = r.cstring()?;
        r.compressed_array(2)?;
        r.skip(4 + 1)?;
        r.compressed_array(4)?;
        let (count, data) = r.compressed_array(2)?;
        let indices = (0..count)
            .map(|i| u16::from_le_bytes([data[i * 2], data[i * 2 + 1]]))
            .collect();
        let weights = r.count()?;
        r.compressed(weights)?;
        selections.push((name, indices));
    }
    for _ in 0..r.count()? {
        r.cstring()?;
        r.cstring()?;
    }
    for _ in 0..r.count()? {
        r.skip(4)?;
        let n = r.count()?;
        r.skip(n * 12)?;
    }
    // Icon colour, selected colour, special flags.
    r.skip(12)?;
    // Vertex data: simple-bone flag, size, clip flags, UV sets, then positions.
    r.skip(1 + 4)?;
    r.compressed_fill(4)?;
    let mut uv_sets = 1;
    let mut i = 0;
    let mut uvs = Vec::new();
    let mut uvs2 = Vec::new();
    while i < uv_sets {
        let range: Vec<f32> = (0..4).map(|_| r.f32()).collect::<Result<_>>()?;
        let n = r.count()?;
        let raw = if r.u8()? != 0 { r.bytes(4)?.repeat(n) } else { r.compressed(n * 4)? };
        let decode = |v: i16, min: f32, max: f32| (v as f32 + 32767.) / 65536. * (max - min) + min;
        let set: Vec<[f32; 2]> = (0..n)
            .map(|k| {
                let u = i16::from_le_bytes([raw[k * 4], raw[k * 4 + 1]]);
                let v = i16::from_le_bytes([raw[k * 4 + 2], raw[k * 4 + 3]]);
                [decode(u, range[0], range[2]), decode(v, range[1], range[3])]
            })
            .collect();
        if i == 0 {
            uvs = set;
            uv_sets = r.count()?.max(1);
        } else if i == 1 {
            uvs2 = set;
        }
        i += 1;
    }
    let (count, data) = r.compressed_array(12)?;
    let f = |o: usize| f32::from_le_bytes(data[o..o + 4].try_into().unwrap());
    let vertices: Vec<[f32; 3]> = (0..count).map(|i| [f(i * 12), f(i * 12 + 4), f(i * 12 + 8)]).collect();
    let normals = read_normals(&mut r, vertices.len()).unwrap_or_default();
    Ok(Lod { resolution, vertices, selections, textures, faces, face_textures, face_materials, materials, uvs, uvs2, normals })
}

/// Normals follow the positions: either three floats each or packed ten bits to a component.
/// Which of the two a file uses is told by the vectors coming out one unit long.
fn read_normals(r: &mut Reader, vertices: usize) -> Result<Vec<[f32; 3]>> {
    let count = r.count()?;
    ensure!(count == vertices && count > 0, "Liczba normalnych ODOL");
    let fill = r.u8()? != 0;
    let start = r.p;
    for size in [12usize, 4] {
        r.p = start;
        let data = if fill {
            match r.bytes(size) {
                Ok(d) => d.repeat(count),
                Err(_) => continue,
            }
        } else {
            match r.compressed(count * size) {
                Ok(d) => d,
                Err(_) => continue,
            }
        };
        let normal = |i: usize| -> [f32; 3] {
            if size == 12 {
                std::array::from_fn(|k| f32::from_le_bytes(data[i * 12 + k * 4..i * 12 + k * 4 + 4].try_into().unwrap()))
            } else {
                let v = u32::from_le_bytes(data[i * 4..i * 4 + 4].try_into().unwrap());
                std::array::from_fn(|k| {
                    let c = ((v >> (10 * k)) & 0x3FF) as i32;
                    (if c > 511 { c - 1024 } else { c }) as f32 / 511.0
                })
            }
        };
        let step = (count / 64).max(1);
        // A few vertices of a model may carry no normal at all.
        let sampled = (0..count).step_by(step).count();
        let unit = (0..count)
            .step_by(step)
            .filter(|&i| {
                let n = normal(i);
                let l = n[0] * n[0] + n[1] * n[1] + n[2] * n[2];
                l.is_finite() && (0.8..1.25).contains(&l)
            })
            .count();
        if unit * 10 >= sampled * 8 {
            return Ok((0..count).map(normal).collect());
        }
    }
    bail!("Nieznany zapis normalnych ODOL")
}

/// The stage holding the colour of a material, by the texture naming convention.
fn diffuse_stage(stages: &[String]) -> Option<String> {
    let has = |suffix: &str| stages.iter().find(|s| {
        let l = s.to_ascii_lowercase();
        !l.starts_with('#') && l.rsplit('.').nth(1).is_some_and(|stem| stem.ends_with(suffix))
    });
    ["_co", "_ca", "_mco", "_mca", "_mc", "_lco"].iter().find_map(|s| has(s)).cloned()
}

/// One texture stage of a material.
#[derive(Debug, Clone, Default)]
pub struct Stage {
    pub texture: String,
    /// Index into `Material::transforms`.
    pub transform: u32,
}

/// UV source and 4x3 matrix (aside, up, dir, pos) of a texture coordinate generator.
#[derive(Debug, Clone, Default)]
pub struct TexGen {
    /// 1 is the first UV set, 5 the second.
    pub source: u32,
    pub matrix: [f32; 12],
}

/// `renderFlags[]` bits of a material.
pub mod render_flag {
    pub const ALWAYS_IN_SHADOW: u32 = 1;
    pub const NO_Z_WRITE: u32 = 1 << 1;
    pub const NO_COLOR_WRITE: u32 = 1 << 4;
    pub const NO_ALPHA_WRITE: u32 = 1 << 5;
    pub const ADD_BLEND: u32 = 1 << 6;
    pub const ALPHA_TEST_32: u32 = 1 << 7;
    pub const ALPHA_TEST_64: u32 = 1 << 8;
    pub const ALPHA_TEST_128: u32 = 1 << 9;
}

/// An RVMAT, as binarised into a model or read from its file.
#[derive(Debug, Clone)]
pub struct Material {
    pub name: String,
    /// Engine number of `PixelShaderID`, see `rvmat::pixel_shader_id`.
    pub pixel_shader: u32,
    pub vertex_shader: u32,
    pub emissive: [f32; 4],
    pub ambient: [f32; 4],
    pub diffuse: [f32; 4],
    pub forced_diffuse: [f32; 4],
    pub specular: [f32; 4],
    pub specular_power: f32,
    /// `render_flag` bits.
    pub render_flags: u32,
    /// `multiTopProjectionLayer` with the angles (degrees from straight up) it fades in over.
    pub top_projection: Option<(u32, f32, f32)>,
    /// Stage 0 is the texture of the face; the others follow the `StageN` classes.
    pub stages: Vec<Stage>,
    pub transforms: Vec<TexGen>,
}
impl Default for Material {
    fn default() -> Self {
        Self {
            name: String::new(),
            pixel_shader: 0,
            vertex_shader: 0,
            emissive: [0.0, 0.0, 0.0, 1.0],
            ambient: [1.0; 4],
            diffuse: [1.0; 4],
            forced_diffuse: [0.0; 4],
            specular: [0.0, 0.0, 0.0, 1.0],
            specular_power: 1.0,
            render_flags: 0,
            top_projection: None,
            stages: Vec::new(),
            transforms: Vec::new(),
        }
    }
}

fn read_material(r: &mut Reader) -> Result<Material> {
    let name = r.cstring()?;
    let version = r.u32()?;
    // Emissive, ambient, diffuse, forced diffuse, specular, a spare, plant wind, specular again.
    let mut colors = [[0.0f32; 4]; 8];
    for c in &mut colors {
        for v in c.iter_mut() {
            *v = r.f32()?;
        }
    }
    let specular_power = r.f32()?;
    // Two words, tree colorization, two more words.
    r.skip(8 + 16 + 8)?;
    let top_layer = r.i32()?;
    let (top_start, top_end) = (r.f32()?, r.f32()?);
    if version >= 20 {
        r.skip(7 * 4)?;
    } else {
        // Older materials end the block before the projection settings.
        r.p -= 20;
    }
    let pixel_shader = r.u32()?;
    let vertex_shader = r.u32()?;
    // Main light and fog mode.
    r.skip(8)?;
    r.cstring()?;
    r.skip(4)?;
    let render_flags = r.u32()?;
    let stage_count = r.count()?;
    let transform_count = r.count()?;
    let mut stages = Vec::with_capacity(stage_count);
    for _ in 0..stage_count {
        r.skip(4)?;
        let texture = r.cstring()?;
        let transform = r.u32()?;
        r.skip(1)?;
        stages.push(Stage { texture, transform });
    }
    let mut transforms = Vec::with_capacity(transform_count);
    for _ in 0..transform_count {
        let source = r.u32()?;
        let mut matrix = [0.0; 12];
        for v in &mut matrix {
            *v = r.f32()?;
        }
        transforms.push(TexGen { source, matrix });
    }
    // Texture stage TI.
    r.skip(4)?;
    r.cstring()?;
    r.skip(5)?;
    let top_projection = (version >= 20 && top_layer >= 0).then_some((top_layer as u32, top_start, top_end));
    Ok(Material {
        name,
        pixel_shader,
        vertex_shader,
        emissive: colors[0],
        ambient: colors[1],
        diffuse: colors[2],
        forced_diffuse: colors[3],
        specular: colors[4],
        specular_power,
        render_flags,
        top_projection,
        stages,
        transforms,
    })
}

/// Resolution of every LOD of an ODOL file, in the file's order.
pub fn resolutions(b: &[u8]) -> Result<Vec<f32>> {
    Ok(lod_table(b)?.1.into_iter().map(|l| l.0).collect())
}

/// The LODs whose resolution `want` accepts, read from an ODOL file.
pub fn read(b: &[u8], want: impl Fn(usize, f32) -> bool) -> Result<Vec<Lod>> {
    let (_, table) = lod_table(b)?;
    let mut lods = Vec::new();
    for (i, &(resolution, start, end)) in table.iter().enumerate() {
        if want(i, resolution) {
            let lod = read_lod(&b[..end], start, resolution)
                .with_context(|| format!("LOD {resolution} modelu ODOL"))?;
            lods.push(lod);
        }
    }
    Ok(lods)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn colour_map_is_found_by_suffix() {
        let m = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(diffuse_stage(&m(&["a_nohq.paa", "b_co.paa", "c_smdi.paa"])), Some("b_co.paa".into()));
        assert_eq!(diffuse_stage(&m(&["a_nohq.paa", "b_mc.paa"])), Some("b_mc.paa".into()));
        assert_eq!(diffuse_stage(&m(&["a_nohq.paa", "#(argb,8,8,3)color(1,1,1,1,co)"])), None);
    }
    #[test]
    fn lzo_literals_matches_and_end_marker() {
        // "abcabcabcabc": literal run "abc", then a 9-byte match at distance 3, end marker.
        let stream = [17 + 3, b'a', b'b', b'c', 32 | 7, (2 << 2) as u8, 0, 17, 0, 0];
        let (out, used) = lzo1x_decompress(&stream, 12).unwrap();
        assert_eq!(out, b"abcabcabcabc");
        assert_eq!(used, stream.len());
        assert!(lzo1x_decompress(&stream, 11).is_err());
    }

    /// Real DayZ models when the P: drive is mounted; skipped otherwise.
    #[test]
    fn reads_track_memory_points_and_vertices_from_dayz_odol() {
        let path = std::path::Path::new("P:/dz/structures/rail/Tracks/Rail_Track_25.p3d");
        let Ok(bytes) = std::fs::read(path) else { return };
        let lods = read(&bytes, |i, r| i == 0 || r == MEMORY).unwrap();
        assert_eq!(lods.len(), 2);
        assert!(lods[0].vertices.len() > 1000);
        let memory = &lods[1];
        let (a, b) = (memory.point("map1").unwrap(), memory.point("map2").unwrap());
        assert!((a[2] - 12.497).abs() < 0.01 && (b[2] + 12.4825).abs() < 0.01, "{a:?} {b:?}");
    }
}
